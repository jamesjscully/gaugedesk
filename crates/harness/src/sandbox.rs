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
//! Network is **deny-by-default** (RF-B3), with three postures (CORE-5):
//! [`Network::Deny`] (isolated, loopback only), [`Network::Filtered`] (egress ONLY
//! to [`SandboxPolicy::allowed_hosts`], enforced by the host-filtering egress
//! proxy in [`crate::egress_proxy`] routed as the netns's sole outbound path), and
//! [`Network::Allow`] (UNFILTERED — reaches any host). `Filtered` is the load-
//! bearing default a non-isolated project runs under: `allowed_hosts` names the
//! model endpoints and nothing else is reachable.
//!
//! **Fail-closed realization (honest boundary).** `Filtered` is only enforceable
//! where the netns can be given a proxy-only outbound path — bubblewrap plus a
//! rootless userspace-net helper (`slirp4netns`/`pasta`). Where that routing is
//! not available (or not yet verified — see [`FILTERED_ROUTING_VERIFIED`]),
//! `Filtered` **degrades to [`Network::Deny`]** (isolated), NEVER silently to
//! unfiltered `Allow`: an unenforceable filter fails to *no* egress, not *open*
//! egress. Declaring hosts via [`SandboxPolicy::allow_hosts`] alone still records
//! *intent* only. Unfiltered egress remains a conscious operator opt-in via
//! [`SandboxPolicy::allow_unfiltered_egress`] (env
//! `GAUGEWRIGHT_ALLOW_UNFILTERED_EGRESS=1`), mirroring `GAUGEWRIGHT_SANDBOX=0`.

use std::path::{Path, PathBuf};

/// Network posture (RF-B3, CORE-5). The policy is **deny-by-default**: a process
/// gets no network unless it declares a need. Three postures, in ascending reach:
///
/// - [`Network::Deny`] — network-isolated. Enforced at the kernel (bubblewrap
///   `--unshare-net`: the namespace has only an empty loopback, so `curl` cannot
///   reach anything). This is what `SandboxPolicy::new` starts at.
/// - [`Network::Filtered`] — egress ONLY to [`SandboxPolicy::allowed_hosts`]. The
///   netns is isolated (`--unshare-net`) and its *sole* outbound path is the
///   host-filtering [`crate::egress_proxy`], which exact-matches the `CONNECT`
///   target host against the allowlist. A host off the list is unreachable even
///   if the agent ignores the proxy env (the netns has no other route). This is
///   the load-bearing posture: `allowed_hosts` becomes the enforced boundary, not
///   mere recorded intent. Realized fail-closed to `Deny` where the routing helper
///   is absent (see [`effective_network`]).
/// - [`Network::Allow`] — the UNFILTERED escape hatch: the namespace shares the
///   host network and can reach *any* host. Set only via
///   [`SandboxPolicy::allow_unfiltered_egress`] (a conscious operator opt-in).
///
/// `allowed_hosts` names the intended egress targets; on its own (via
/// [`SandboxPolicy::allow_hosts`]) it records intent without opening the network —
/// only [`SandboxPolicy::filter_egress`] (→ `Filtered`) or
/// [`SandboxPolicy::allow_unfiltered_egress`] (→ `Allow`) changes the posture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Network {
    Allow,
    Filtered,
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

    /// Request **filtered** egress (CORE-5): the process may reach `hosts` and
    /// *only* `hosts`, enforced by the host-filtering [`crate::egress_proxy`]
    /// routed as the isolated netns's sole outbound path. Unlike
    /// [`Self::allow_hosts`], this DOES change the posture — to [`Network::Filtered`]
    /// — because the allowlist is now load-bearing, not just recorded intent. It is
    /// still fail-closed: where the netns routing helper is unavailable the posture
    /// realizes as [`Network::Deny`] (isolated), never as unfiltered `Allow` (see
    /// [`effective_network`]). This is the posture a non-isolated project runs
    /// under; unfiltered egress stays the separate conscious opt-in
    /// ([`Self::allow_unfiltered_egress`]).
    pub fn filter_egress(mut self, hosts: Vec<String>) -> Self {
        self.allowed_hosts = hosts;
        self.network = Network::Filtered;
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

/// Whether the netns→proxy last-mile routing for [`Network::Filtered`] is wired
/// **and verified non-bypassable on a real `slirp4netns`/`pasta` host**.
///
/// The enforcement heart — the host-filtering [`crate::egress_proxy`] and the
/// whole policy/posture path — is built and tested. The remaining piece is
/// attaching the userspace-net helper to the spawned bubblewrap process's netns
/// and default-dropping every route except the proxy (the design of record is in
/// ADR 0079). That attach cannot be verified without a `slirp4netns`/`pasta` host,
/// and an *unverified* egress filter that silently leaks would be worse than
/// honest isolation. So until it is landed and verified, this stays `false` and
/// `Filtered` realizes **fail-closed to [`Network::Deny`]** (isolated) — see
/// [`effective_network`]. Flip to `true` only together with the verified attach.
pub const FILTERED_ROUTING_VERIFIED: bool = false;

/// Host capabilities that decide whether [`Network::Filtered`] can be *enforced*
/// (CORE-5). Filtered needs a bubblewrap netns plus a rootless userspace-net
/// helper to give that netns a proxy-only outbound path; without both there is no
/// non-bypassable route, so Filtered must fail closed to isolation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RoutingCaps {
    /// bubblewrap is available to create the isolated netns.
    pub bwrap: bool,
    /// The rootless outbound helper on PATH (`pasta` preferred, else `slirp4netns`),
    /// or `None` if neither is present.
    pub userspace_net: Option<&'static str>,
}

impl RoutingCaps {
    /// Can [`Network::Filtered`] be enforced end-to-end here? Requires the netns
    /// backend, the outbound helper, AND the verified last-mile routing.
    pub fn can_enforce_filtered(&self) -> bool {
        FILTERED_ROUTING_VERIFIED && self.bwrap && self.userspace_net.is_some()
    }
}

/// Probe this host for the [`Network::Filtered`] enforcement capabilities. Linux
/// only (bubblewrap + `slirp4netns`/`pasta` are the rootless path); every other OS
/// reports no capability, so `Filtered` fails closed to isolation there.
pub fn detect_routing_caps() -> RoutingCaps {
    #[cfg(target_os = "linux")]
    {
        let userspace_net = if program_on_path("pasta") {
            Some("pasta")
        } else if program_on_path("slirp4netns") {
            Some("slirp4netns")
        } else {
            None
        };
        RoutingCaps {
            bwrap: program_on_path("bwrap"),
            userspace_net,
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        RoutingCaps {
            bwrap: false,
            userspace_net: None,
        }
    }
}

/// Resolve the **effective** network posture from the requested one and this
/// host's capabilities (CORE-5 fail-closed rule). Pure so the precedence is
/// unit-testable:
///
/// - `Allow` and `Deny` pass through unchanged.
/// - `Filtered` stays `Filtered` **only** when [`RoutingCaps::can_enforce_filtered`]
///   holds; otherwise it degrades to `Deny` (isolated). It NEVER degrades to
///   `Allow` — an unenforceable filter fails to *no* egress, not *open* egress.
pub fn effective_network(requested: Network, caps: RoutingCaps) -> Network {
    match requested {
        Network::Filtered if !caps.can_enforce_filtered() => Network::Deny,
        other => other,
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
        // Isolate the netns for both `Deny` (fully isolated) and `Filtered`
        // (isolated, with the host-filtering egress proxy attached out-of-band as
        // its sole outbound path — ADR 0079). Only the unfiltered `Allow` opt-in
        // shares the host network. The Filtered→Deny fail-closed degrade
        // ([`effective_network`]) produces the same `--unshare-net` argv, so an
        // unenforceable filter is byte-for-byte an isolated run.
        if policy.network != Network::Allow {
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
        // macOS Seatbelt cannot host the Linux netns-proxy routing that `Filtered`
        // needs (no rootless userspace-net + isolated netns to pin outbound to the
        // proxy), and SBPL `network*` filters by address/port, not by TLS SNI/host —
        // so there is no honest per-host filter to write here. `Filtered` therefore
        // fails closed to isolated on macOS: deny network for both `Deny` and
        // `Filtered`; only the unfiltered `Allow` opt-in leaves it open.
        if policy.network != Network::Allow {
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
            // Resolve the effective posture for THIS host (CORE-5 fail-closed):
            // `Filtered` realizes as `Deny` (isolated) where the netns routing
            // helper is unavailable/unverified — never as unfiltered `Allow`.
            let effective = effective_network(policy.network, detect_routing_caps());
            // Observability (RF-A8): record the enforcement decision —
            // backend name + posture only, never the worktree contents.
            tracing::info!(
                backend = backend.name(),
                requested_network = ?policy.network,
                effective_network = ?effective,
                read_only_roots = policy.read_only_roots.len(),
                "pi spawn: sandboxed"
            );
            if policy.network == Network::Filtered && effective == Network::Deny {
                // Honest, loud: the operator asked for filtered egress but this host
                // can't enforce it, so the run is network-isolated (the model
                // endpoint is unreachable) rather than opened wide.
                eprintln!(
                    "gaugewright: filtered egress requested but not enforceable here \
                     (needs bubblewrap + slirp4netns/pasta{}); failing CLOSED to \
                     network-isolated. Set GAUGEWRIGHT_ALLOW_UNFILTERED_EGRESS=1 to \
                     consciously allow UNFILTERED egress instead.",
                    if FILTERED_ROUTING_VERIFIED {
                        ""
                    } else {
                        ", and verified netns routing (not yet landed)"
                    }
                );
            }
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
    fn filter_egress_sets_the_filtered_posture_and_records_the_allowlist() {
        // CORE-5: `filter_egress` is the load-bearing builder — unlike `allow_hosts`
        // it changes the posture, because the allowlist is now enforced.
        let p = SandboxPolicy::new(vec![PathBuf::from("/wt")])
            .filter_egress(vec!["api.openai.com".into(), "chatgpt.com".into()]);
        assert_eq!(p.network, Network::Filtered);
        assert_eq!(
            p.allowed_hosts,
            vec!["api.openai.com".to_string(), "chatgpt.com".to_string()]
        );
        // The unfiltered opt-in still wins over a filtered request (conscious escalation).
        let unfiltered = SandboxPolicy::new(vec![PathBuf::from("/wt")])
            .filter_egress(vec!["api.openai.com".into()])
            .allow_unfiltered_egress(true);
        assert_eq!(unfiltered.network, Network::Allow);
    }

    #[test]
    fn effective_network_fails_closed_from_filtered_to_deny_never_to_allow() {
        // Allow and Deny always pass through unchanged.
        let full = RoutingCaps {
            bwrap: true,
            userspace_net: Some("slirp4netns"),
        };
        assert_eq!(effective_network(Network::Allow, full), Network::Allow);
        assert_eq!(effective_network(Network::Deny, full), Network::Deny);

        // Filtered is enforceable ONLY when caps allow AND the routing is verified.
        // The gate is tied to the flag so this stays honest when the flag flips.
        assert_eq!(full.can_enforce_filtered(), FILTERED_ROUTING_VERIFIED);
        let expected_full = if FILTERED_ROUTING_VERIFIED {
            Network::Filtered
        } else {
            Network::Deny // the honest current reality: routing not yet verified
        };
        assert_eq!(effective_network(Network::Filtered, full), expected_full);

        // Missing either capability ⇒ Filtered fails CLOSED to Deny, never Allow.
        let no_helper = RoutingCaps {
            bwrap: true,
            userspace_net: None,
        };
        let no_bwrap = RoutingCaps {
            bwrap: false,
            userspace_net: Some("pasta"),
        };
        assert_eq!(
            effective_network(Network::Filtered, no_helper),
            Network::Deny
        );
        assert_eq!(
            effective_network(Network::Filtered, no_bwrap),
            Network::Deny
        );
    }

    #[test]
    fn bubblewrap_isolates_the_netns_for_filtered_and_shares_it_only_for_allow() {
        // Filtered: the netns is isolated (`--unshare-net`) — the egress proxy is
        // its only outbound path, attached out-of-band. Same argv as Deny, so an
        // unenforceable filter is byte-for-byte an isolated run.
        let filtered = SandboxPolicy::new(vec![PathBuf::from("/wt")])
            .filter_egress(vec!["api.openai.com".into()]);
        let argv = Bubblewrap.wrap(&filtered, "pi", &[], None).unwrap();
        assert!(
            argv.iter().any(|a| a == "--unshare-net"),
            "Filtered must isolate the netns (proxy is its sole route)"
        );
        // Only the unfiltered Allow opt-in shares the host network.
        let allow = SandboxPolicy::new(vec![PathBuf::from("/wt")])
            .filter_egress(vec!["api.openai.com".into()])
            .allow_unfiltered_egress(true);
        let argv = Bubblewrap.wrap(&allow, "pi", &[], None).unwrap();
        assert!(
            !argv.iter().any(|a| a == "--unshare-net"),
            "unfiltered Allow shares the host network"
        );
    }

    #[test]
    fn seatbelt_denies_network_for_filtered_failing_closed_on_macos() {
        // macOS can't host the netns-proxy routing, so Filtered fails closed to
        // isolated: the profile denies network (same as Deny).
        let filtered = SandboxPolicy::new(vec![PathBuf::from("/wt")])
            .filter_egress(vec!["api.openai.com".into()]);
        assert!(Seatbelt::profile(&filtered).contains("(deny network*)"));
        // The unfiltered opt-in leaves the network open (no deny line).
        let allow = SandboxPolicy::new(vec![PathBuf::from("/wt")])
            .filter_egress(vec!["api.openai.com".into()])
            .allow_unfiltered_egress(true);
        assert!(!Seatbelt::profile(&allow).contains("(deny network*)"));
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
