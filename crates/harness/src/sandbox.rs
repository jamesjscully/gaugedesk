//! OS sandbox that wraps the Pi subprocess (ADR 0030).
//!
//! Boundary enforcement that an in-process membrane cannot provide: a `bash`
//! tool (or any exec tool) spawns a child with the same OS authority, whose
//! syscalls gaugewright never sees. The fix is to run the **whole `pi --mode rpc`
//! process** — and therefore every child it spawns — under an OS sandbox. One
//! uniform [`SandboxPolicy`]; per-OS [`Sandbox`] backends.
//!
//! The key property: the agent's method-definition surface is passed as a
//! `read_only_root` *inside* the writable worktree, so `bash`, `edit`, `write`,
//! `chmod`, and `rm` all fail at the kernel uniformly (`INV-24`). `bash` keeps
//! full power everywhere else. Filesystem integrity + workspace confinement are
//! what this discharges.
//!
//! Network is **deny-by-default** (RF-B3). Declaring hosts via
//! [`SandboxPolicy::allow_hosts`] records *intent* but does NOT open the kernel
//! network: with no per-host egress proxy yet (deferred infra — the kernel can
//! only deny *all* or allow *all*), opening it would be silent UNFILTERED egress
//! (a compromised/prompt-injected agent could exfiltrate protected method+context
//! to any host). So the posture stays isolated (`--unshare-net`) until the
//! operator explicitly accepts unfiltered egress via
//! [`SandboxPolicy::allow_unfiltered_egress`] (env `GAUGEWRIGHT_ALLOW_UNFILTERED_EGRESS=1`),
//! mirroring the conscious `GAUGEWRIGHT_SANDBOX=0` opt-out.

use std::path::{Path, PathBuf};

/// Network posture (RF-B3). The policy is **deny-by-default**: a process gets no
/// network unless it declares a need. `Deny` with no `allowed_hosts` is enforced
/// at the kernel (bubblewrap `--unshare-net`: the namespace has only an empty
/// loopback, so `curl` cannot reach anything). `Allow` is the un-filtered escape
/// hatch; `allowed_hosts` names the *intended* egress targets (e.g. the model
/// endpoint) for the per-host allowlist proxy. Host-level filtering among allowed
/// targets needs that proxy routing the namespace's traffic — the one piece that
/// is deferred infra (a userspace-net helper); the kernel can only deny *all* or
/// allow *all*. So `allowed_hosts` only records the *intended* targets; the
/// posture flips to `Allow` (kernel network not isolated) **only** when the
/// operator explicitly accepts unfiltered egress via
/// [`SandboxPolicy::allow_unfiltered_egress`] — never silently from declaring hosts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Network {
    Allow,
    Deny,
}

/// A uniform sandbox policy, mapped to per-OS backends.
#[derive(Clone, Debug)]
pub struct SandboxPolicy {
    /// Roots the process may write (e.g. the engagement worktree, the Pi config
    /// dir). Everything else is read-only.
    pub writable_roots: Vec<PathBuf>,
    /// Paths re-imposed read-only *on top of* the writable roots — the agent's
    /// definition surface in use mode. Must exist on disk (a bind needs a source).
    pub read_only_roots: Vec<PathBuf>,
    pub network: Network,
    /// The egress targets this process is *intended* to reach (e.g. the model
    /// endpoint host). Empty under `Deny` ⇒ fully network-isolated. Non-empty ⇒
    /// the allowlist the per-host proxy enforces (RF-B3); recorded here so the
    /// allowance is explicit and auditable, never ambient.
    pub allowed_hosts: Vec<String>,
}

impl SandboxPolicy {
    /// A **deny-by-default** policy (RF-B3): writable roots, no network. Add
    /// network deliberately with [`Self::allow_hosts`].
    pub fn new(writable_roots: Vec<PathBuf>) -> Self {
        Self {
            writable_roots,
            read_only_roots: Vec::new(),
            network: Network::Deny,
            allowed_hosts: Vec::new(),
        }
    }
    pub fn read_only(mut self, roots: Vec<PathBuf>) -> Self {
        self.read_only_roots = roots;
        self
    }
    /// Declare the egress targets this process needs (the model endpoint, etc.).
    /// **Records intent only** — it does NOT open the kernel network. Without the
    /// per-host egress proxy (deferred infra) the kernel can only deny *all* or
    /// allow *all*, so opening the network from a declared host would be silent
    /// UNFILTERED egress. The posture therefore stays `Deny` (network-isolated,
    /// fail-closed) until the operator explicitly accepts unfiltered egress via
    /// [`Self::allow_unfiltered_egress`]; the hosts are kept auditable for when the
    /// proxy lands.
    pub fn allow_hosts(mut self, hosts: Vec<String>) -> Self {
        self.allowed_hosts = hosts;
        self
    }

    /// Explicitly accept **unfiltered** network egress (RF-B3). With no per-host
    /// proxy yet, the kernel cannot filter to [`Self::allowed_hosts`], so allowing
    /// egress means the process can reach *any* host — a real exfiltration surface
    /// for a compromised or prompt-injected agent. This is therefore the conscious
    /// operator opt-in (env `GAUGEWRIGHT_ALLOW_UNFILTERED_EGRESS=1`), mirroring the
    /// `GAUGEWRIGHT_SANDBOX=0` seam: only when `acknowledged` does the posture flip to
    /// `Allow` (kernel network not isolated). Without it the process stays
    /// network-isolated, so a declared-but-unacknowledged egress need fails closed.
    pub fn allow_unfiltered_egress(mut self, acknowledged: bool) -> Self {
        if acknowledged {
            self.network = Network::Allow;
        }
        self
    }
}

/// A per-OS enforcement backend.
pub trait Sandbox {
    fn name(&self) -> &'static str;
    /// Build the full argv that runs `program args…` under `policy` with cwd `cwd`.
    /// `None` means this backend can't wrap here — the caller runs Pi unwrapped
    /// (with a visible warning; never a silent downgrade).
    fn wrap(
        &self,
        policy: &SandboxPolicy,
        program: &str,
        args: &[String],
        cwd: Option<&Path>,
    ) -> Option<Vec<String>>;
}

/// Linux: bubblewrap. `--ro-bind / /` makes the host read-only, writable roots are
/// layered with `--bind`, then read-only roots are re-imposed with `--ro-bind`
/// *after* (later binds win) so the definition surface inside the worktree is
/// read-only even to `bash`.
pub struct Bubblewrap;

impl Sandbox for Bubblewrap {
    fn name(&self) -> &'static str {
        "bubblewrap"
    }
    fn wrap(
        &self,
        policy: &SandboxPolicy,
        program: &str,
        args: &[String],
        cwd: Option<&Path>,
    ) -> Option<Vec<String>> {
        let mut v: Vec<String> = vec!["bwrap".into(), "--die-with-parent".into()];
        // Read-only host, with a real writable /tmp and minimal /dev + /proc.
        v.extend(["--ro-bind", "/", "/"].map(String::from));
        v.extend(["--dev", "/dev"].map(String::from));
        v.extend(["--proc", "/proc"].map(String::from));
        v.extend(["--bind", "/tmp", "/tmp"].map(String::from));
        for w in &policy.writable_roots {
            v.push("--bind".into());
            v.push(w.to_string_lossy().into_owned());
            v.push(w.to_string_lossy().into_owned());
        }
        // After the writable binds, so a definition path nested in the worktree wins.
        for r in &policy.read_only_roots {
            v.push("--ro-bind".into());
            v.push(r.to_string_lossy().into_owned());
            v.push(r.to_string_lossy().into_owned());
        }
        if policy.network == Network::Deny {
            v.push("--unshare-net".into());
        }
        if let Some(cwd) = cwd {
            v.push("--chdir".into());
            v.push(cwd.to_string_lossy().into_owned());
        }
        v.push("--".into());
        v.push(program.into());
        v.extend(args.iter().cloned());
        Some(v)
    }
}

/// macOS: Seatbelt via `sandbox-exec -p <SBPL>`. Allow-by-default, deny all
/// writes, re-allow the writable roots, then re-deny the definition surface last.
pub struct Seatbelt;

impl Seatbelt {
    /// Build the Sandbox Profile Language source for a policy (pure; testable on
    /// any OS).
    pub fn profile(policy: &SandboxPolicy) -> String {
        let mut p =
            String::from("(version 1)\n(allow default)\n(deny file-write* (subpath \"/\"))\n");
        for w in &policy.writable_roots {
            p.push_str(&format!(
                "(allow file-write* (subpath \"{}\"))\n",
                w.to_string_lossy()
            ));
        }
        for r in &policy.read_only_roots {
            p.push_str(&format!(
                "(deny file-write* (subpath \"{}\"))\n",
                r.to_string_lossy()
            ));
        }
        if policy.network == Network::Deny {
            p.push_str("(deny network*)\n");
        }
        p
    }
}

impl Sandbox for Seatbelt {
    fn name(&self) -> &'static str {
        "seatbelt"
    }
    fn wrap(
        &self,
        policy: &SandboxPolicy,
        program: &str,
        args: &[String],
        _cwd: Option<&Path>,
    ) -> Option<Vec<String>> {
        let mut v = vec![
            "sandbox-exec".into(),
            "-p".into(),
            Self::profile(policy),
            program.into(),
        ];
        v.extend(args.iter().cloned());
        Some(v)
    }
}

/// Windows: AppContainer / restricted token with a deny-write ACE on the
/// definition surface (ADR 0030). **Deferred — needs a Windows build/CI host**
/// (RF-B2): unlike the Linux/macOS backends, there is no CLI wrapper to shell out
/// to, so this must be a Win32 FFI backend and cannot be built or verified on the
/// Linux-only toolchain/CI this project runs. `wrap` returns `None`, which is now
/// **safe** because [`PiProcess::spawn`](crate::PiProcess::spawn) fails closed
/// when a protected definition surface cannot be sandboxed (RF-B1) — so the
/// Windows hole is shut today; this backend is the *enforcement* that lets
/// use-mode actually run on Windows.
///
/// Design when a Windows host is available:
/// - create a per-engagement **AppContainer profile** (a capability SID), and
///   launch Pi with `CreateProcess` + `STARTUPINFOEX`/`PROC_THREAD_ATTRIBUTE_*`
///   carrying the AppContainer SID and an explicit (empty/minimal) capability set;
/// - grant the AppContainer SID write access only to `writable_roots` (ACLs), and
///   add an explicit **deny-write ACE** for the AppContainer SID on each
///   `read_only_roots` path so the method-definition surface is unwritable even to
///   `bash`/PowerShell children (the INV-24 property the Linux backend gets from
///   `--ro-bind`);
/// - map `Network::Deny` to withholding the `internetClient` capability (no
///   outbound sockets); a non-empty `allowed_hosts` keeps the capability and
///   leaves per-host filtering to the egress proxy, as on the other backends.
pub struct WindowsSandbox;

impl Sandbox for WindowsSandbox {
    fn name(&self) -> &'static str {
        "windows-appcontainer(planned)"
    }
    fn wrap(
        &self,
        _policy: &SandboxPolicy,
        _program: &str,
        _args: &[String],
        _cwd: Option<&Path>,
    ) -> Option<Vec<String>> {
        None
    }
}

/// No sandbox — runs the process unwrapped. The honest fallback when no backend
/// is available or `GAUGEWRIGHT_SANDBOX=0`; the caller logs that the run is unsandboxed.
pub struct NoSandbox;

impl Sandbox for NoSandbox {
    fn name(&self) -> &'static str {
        "none"
    }
    fn wrap(
        &self,
        _policy: &SandboxPolicy,
        _program: &str,
        _args: &[String],
        _cwd: Option<&Path>,
    ) -> Option<Vec<String>> {
        None
    }
}

/// Pick the backend for this host. `GAUGEWRIGHT_SANDBOX=0` forces [`NoSandbox`].
pub fn detect() -> Box<dyn Sandbox> {
    if std::env::var("GAUGEWRIGHT_SANDBOX").as_deref() == Ok("0") {
        return Box::new(NoSandbox);
    }
    #[cfg(target_os = "linux")]
    {
        if program_on_path("bwrap") {
            return Box::new(Bubblewrap);
        }
        return Box::new(NoSandbox);
    }
    #[cfg(target_os = "macos")]
    {
        return Box::new(Seatbelt); // sandbox-exec ships with macOS
    }
    #[cfg(target_os = "windows")]
    {
        return Box::new(WindowsSandbox);
    }
    #[allow(unreachable_code)]
    Box::new(NoSandbox)
}

/// Wrap `program args…` (and every child it spawns, incl. `bash`) in an OS
/// sandbox so the definition surface is read-only at the kernel (ADR 0030). A
/// backend that can't wrap here either fails closed (a protected definition
/// surface must not run unenforced — RF-B1) or, with no protected surface or an
/// explicit opt-out, falls back to an unwrapped but loudly flagged run.
pub fn wrap_or_refuse(
    policy: &SandboxPolicy,
    program: &str,
    args: &[String],
    cwd: Option<&Path>,
) -> std::io::Result<std::process::Command> {
    let backend = detect();
    match backend.wrap(policy, program, args, cwd) {
        Some(argv) => {
            // Observability (RF-A8): record the enforcement decision —
            // backend name + posture only, never the worktree contents.
            tracing::info!(
                backend = backend.name(),
                network = ?policy.network,
                read_only_roots = policy.read_only_roots.len(),
                "pi spawn: sandboxed"
            );
            let mut c = std::process::Command::new(&argv[0]);
            c.args(&argv[1..]);
            Ok(c)
        }
        None => {
            let explicit_optout = std::env::var("GAUGEWRIGHT_SANDBOX").as_deref() == Ok("0");
            if !allow_unsandboxed(policy, explicit_optout) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    format!(
                        "sandbox backend '{}' cannot wrap here, and the policy \
                         protects a method-definition surface (INV-24): refusing \
                         to run unsandboxed. Install a backend (bwrap on Linux) \
                         or set GAUGEWRIGHT_SANDBOX=0 to explicitly accept an \
                         unenforced run.",
                        backend.name()
                    ),
                ));
            }
            eprintln!(
                "gaugewright: sandbox backend '{}' unavailable — running Pi UNSANDBOXED \
                 ({}; install a backend to enforce)",
                backend.name(),
                if explicit_optout {
                    "GAUGEWRIGHT_SANDBOX=0"
                } else {
                    "no protected definition surface in this policy"
                }
            );
            let mut c = std::process::Command::new(program);
            c.args(args);
            Ok(c)
        }
    }
}

/// May a spawn proceed *unsandboxed* under `policy` (RF-B1)? Fail closed when
/// the policy re-imposes a read-only definition surface — that is the use-mode
/// case where the OS sandbox is the load-bearing INV-24 enforcement, so running
/// without it would let the agent rewrite its own method. An explicit
/// `GAUGEWRIGHT_SANDBOX=0` opt-out is the one override: the operator has consciously
/// accepted an unenforced run (dev/test), which is a decision, not a downgrade.
fn allow_unsandboxed(policy: &SandboxPolicy, explicit_optout: bool) -> bool {
    policy.read_only_roots.is_empty() || explicit_optout
}

/// Is `name` an executable on `PATH`? (No external `which` dependency.)
#[cfg(target_os = "linux")]
fn program_on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .map(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> SandboxPolicy {
        SandboxPolicy::new(vec![PathBuf::from("/home/u/wt")]).read_only(vec![
            PathBuf::from("/home/u/wt/.pi"),
            PathBuf::from("/home/u/wt/AGENTS.md"),
        ])
    }

    #[test]
    fn bubblewrap_layers_readonly_definition_over_writable_worktree() {
        let argv = Bubblewrap
            .wrap(
                &policy(),
                "pi",
                &["--mode".into(), "rpc".into()],
                Some(Path::new("/home/u/wt")),
            )
            .unwrap();
        let joined = argv.join(" ");
        // host read-only, worktree writable, definition re-imposed read-only.
        assert!(joined.contains("--ro-bind / /"));
        assert!(joined.contains("--bind /home/u/wt /home/u/wt"));
        assert!(joined.contains("--ro-bind /home/u/wt/.pi /home/u/wt/.pi"));
        assert!(joined.contains("--ro-bind /home/u/wt/AGENTS.md /home/u/wt/AGENTS.md"));
        // the writable bind comes BEFORE the read-only re-impose (later wins).
        let bind = joined.find("--bind /home/u/wt /home/u/wt").unwrap();
        let robind = joined.find("--ro-bind /home/u/wt/.pi").unwrap();
        assert!(
            bind < robind,
            "writable worktree must be bound before the RO definition"
        );
        // the wrapped program follows `--`.
        let dd = argv.iter().position(|a| a == "--").unwrap();
        assert_eq!(&argv[dd + 1..], &["pi", "--mode", "rpc"]);
        // chdir into the worktree.
        assert!(joined.contains("--chdir /home/u/wt"));
    }

    #[test]
    fn bubblewrap_edit_mode_has_no_readonly_definition() {
        let p = SandboxPolicy::new(vec![PathBuf::from("/home/u/wt")]); // no read_only_roots
        let argv = Bubblewrap.wrap(&p, "pi", &[], None).unwrap();
        assert!(!argv.join(" ").contains("--ro-bind /home/u/wt"));
    }

    #[test]
    fn seatbelt_profile_allows_worktree_then_denies_definition() {
        let prof = Seatbelt::profile(&policy());
        assert!(prof.contains("(deny file-write* (subpath \"/\"))"));
        let allow = prof
            .find("(allow file-write* (subpath \"/home/u/wt\"))")
            .unwrap();
        let deny = prof
            .find("(deny file-write* (subpath \"/home/u/wt/.pi\"))")
            .unwrap();
        assert!(
            allow < deny,
            "writable allow must precede the definition re-deny"
        );
    }

    #[test]
    fn network_stays_isolated_until_unfiltered_egress_is_acknowledged() {
        // RF-B3: a fresh policy denies network — bubblewrap unshares the net.
        let denied = SandboxPolicy::new(vec![PathBuf::from("/wt")]);
        assert_eq!(denied.network, Network::Deny);
        assert!(denied.allowed_hosts.is_empty());
        let argv = Bubblewrap.wrap(&denied, "pi", &[], None).unwrap();
        assert!(
            argv.iter().any(|a| a == "--unshare-net"),
            "deny-by-default must unshare the network namespace"
        );

        // Declaring hosts records INTENT but must NOT open the kernel network —
        // without a per-host proxy that would be silent unfiltered egress (M-1).
        let declared = SandboxPolicy::new(vec![PathBuf::from("/wt")])
            .allow_hosts(vec!["api.openai.com".into()]);
        assert_eq!(
            declared.network,
            Network::Deny,
            "declaring hosts must not silently open egress"
        );
        assert_eq!(declared.allowed_hosts, vec!["api.openai.com".to_string()]);
        let argv = Bubblewrap.wrap(&declared, "pi", &[], None).unwrap();
        assert!(
            argv.iter().any(|a| a == "--unshare-net"),
            "a declared-but-unacknowledged egress need stays network-isolated (fail-closed)"
        );

        // Only an explicit unfiltered-egress acknowledgment opens the namespace.
        let acknowledged = SandboxPolicy::new(vec![PathBuf::from("/wt")])
            .allow_hosts(vec!["api.openai.com".into()])
            .allow_unfiltered_egress(true);
        assert_eq!(acknowledged.network, Network::Allow);
        let argv = Bubblewrap.wrap(&acknowledged, "pi", &[], None).unwrap();
        assert!(
            !argv.iter().any(|a| a == "--unshare-net"),
            "an acknowledged egress need opens the namespace network"
        );

        // The acknowledgment is opt-in: `false` leaves the process isolated.
        let not_ack = SandboxPolicy::new(vec![PathBuf::from("/wt")])
            .allow_hosts(vec!["api.openai.com".into()])
            .allow_unfiltered_egress(false);
        assert_eq!(not_ack.network, Network::Deny);
    }

    #[test]
    fn nosandbox_and_windows_stub_return_unwrapped() {
        assert!(NoSandbox.wrap(&policy(), "pi", &[], None).is_none());
        assert!(WindowsSandbox.wrap(&policy(), "pi", &[], None).is_none());
    }

    /// The real property, end-to-end: a `bash`-style write to a read-only root
    /// fails at the kernel, while a write to a writable root succeeds. Skips where
    /// user namespaces / `bwrap` aren't usable (some CI sandboxes).
    #[cfg(target_os = "linux")]
    #[test]
    fn bubblewrap_blocks_bash_writes_to_readonly_roots() {
        use std::process::Command as PC;
        if !program_on_path("bwrap") {
            eprintln!("skip: bwrap absent");
            return;
        }
        if !matches!(PC::new("bwrap").args(["--ro-bind", "/", "/", "--", "true"]).status(),
            Ok(s) if s.success())
        {
            eprintln!("skip: bwrap unusable here (no user namespaces)");
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let wt = dir.path();
        std::fs::create_dir(wt.join(".pi")).unwrap();
        std::fs::write(wt.join(".pi/SYSTEM.md"), "ORIGINAL").unwrap();

        // worktree writable, the definition surface read-only on top (use mode).
        let policy = SandboxPolicy::new(vec![wt.to_path_buf()]).read_only(vec![wt.join(".pi")]);
        // bash tries to rewrite its own system prompt AND write an ordinary file.
        let script =
            "echo HACKED > .pi/SYSTEM.md 2>/dev/null; echo ok > work.txt 2>/dev/null; true";
        let argv = Bubblewrap
            .wrap(&policy, "/bin/sh", &["-c".into(), script.into()], Some(wt))
            .unwrap();
        let status = PC::new(&argv[0]).args(&argv[1..]).status().unwrap();
        assert!(status.success());

        // INV-24: the protected system prompt is unchanged…
        assert_eq!(
            std::fs::read_to_string(wt.join(".pi/SYSTEM.md")).unwrap(),
            "ORIGINAL",
            "bash must not be able to rewrite the read-only definition"
        );
        // …but ordinary work in the writable worktree went through (bash unharmed).
        assert!(
            wt.join("work.txt").exists(),
            "writable-root write should succeed"
        );
    }

    /// RF-B1: when no backend can wrap, a policy that re-imposes a read-only
    /// definition surface (use mode — INV-24 load-bearing) must refuse to run
    /// unsandboxed; only an explicit `GAUGEWRIGHT_SANDBOX=0` opt-out or a policy with
    /// no protected surface (edit mode) may warn-and-run.
    #[test]
    fn unsandboxed_run_fails_closed_when_definition_surface_is_protected() {
        let protected = SandboxPolicy::new(vec!["/wt".into()]).read_only(vec!["/wt/.pi".into()]);
        let unprotected = SandboxPolicy::new(vec!["/wt".into()]);
        assert!(
            !allow_unsandboxed(&protected, false),
            "use mode without an explicit opt-out must fail closed"
        );
        assert!(
            allow_unsandboxed(&protected, true),
            "GAUGEWRIGHT_SANDBOX=0 is a conscious operator decision"
        );
        assert!(
            allow_unsandboxed(&unprotected, false),
            "edit mode (no protected surface) keeps the warn-and-run fallback"
        );
    }

    #[test]
    fn detect_respects_the_disable_override() {
        // Save/restore the env so the test is hermetic.
        let prev = std::env::var("GAUGEWRIGHT_SANDBOX").ok();
        std::env::set_var("GAUGEWRIGHT_SANDBOX", "0");
        assert_eq!(detect().name(), "none");
        match prev {
            Some(v) => std::env::set_var("GAUGEWRIGHT_SANDBOX", v),
            None => std::env::remove_var("GAUGEWRIGHT_SANDBOX"),
        }
    }
}
