//! gaugewright boundary — the egress membrane + `.agent-config.json` policy.
//!
//! The host-side chokepoint every external effect passes through (`pi-rpc.md`,
//! "Egress Mediation"). It is **not** a bypass: even trust-by-default mediates
//! every effect — the posture only sets the *default decision* to
//! allow-and-record vs. block. The pure soundness of release (conjunctive
//! consent, `INV-22`) lives in [`gaugewright_core::boundary`]; this crate is the
//! imperative policy gate that classifies an effect against the agent's declared
//! policy before it executes.
//!
//! The policy is the original `.agent-config.json` `policy` block
//! (read/write/execute/egress, `builder_only/**` hidden), adopted directly for M0.

use std::collections::BTreeSet;

use serde::Deserialize;

/// Which engagement mode a turn runs in, for the method-definition write-gate
/// (`INV-24`, ADR 0029). `Edit` may edit the agent's own definition; `Use` is
/// read-only to it. Defaults to `Use` (fail-closed: a context that forgot to
/// declare edit cannot rewrite the agent).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AuthoringMode {
    Edit,
    #[default]
    Use,
}

/// The agent's **method-definition surface** — the Pi-native files that define the
/// agent (ADR 0029). A write here is edit-authored (`INV-24`).
pub fn is_method_surface_path(path: &str) -> bool {
    let p = path.trim_start_matches("./");
    p == ".agent-config.json"
        || p == "AGENTS.md"
        || p == "CLAUDE.md"
        || p.starts_with(".pi/")
        || p.contains("/.pi/")
        || p.ends_with("/AGENTS.md")
        || p.ends_with("/CLAUDE.md")
        || p.ends_with("/.agent-config.json")
}

/// Pi's built-in file-mutating tools. This membrane gate gives a fast, clean
/// rejection for these (defense-in-depth + audit); the load-bearing INV-24
/// enforcement is the OS sandbox, which makes the surface read-only for *every*
/// write path including a `bash` redirection ([ADR 0030]). So no tool — `bash`
/// included — is special-cased or blocked to protect the definition.
///
/// [ADR 0030]: ../../../specs/decisions/0030-sandbox-as-boundary-enforcement.md
pub fn is_write_tool(tool: &str) -> bool {
    matches!(tool, "write" | "edit")
}

/// The agent **definition surface** — the one home of the on-disk layout that
/// defines an agent (ADR 0029). Every Rust consumer (seeding, sandbox read-only
/// roots, config reads) derives from these constants; the TS plugin and the web
/// client keep cross-language copies annotated back to this module. The layout
/// itself is Pi-native versioned product content and does not move until SUB-3
/// — this module makes it one decision instead of scattered copies. Turn-time
/// persona/config discovery of the layout is an adapter obligation until SUB-3
/// (the Pi runtime discovers [`definition::SYSTEM_PATH`] from its cwd).
pub mod definition {
    /// The agent's persona file.
    pub const SYSTEM_PATH: &str = ".pi/SYSTEM.md";
    /// The agent's working instructions/conventions file.
    pub const INSTRUCTIONS_PATH: &str = "AGENTS.md";
    /// The agent's model + policy config (the membrane's policy source).
    pub const CONFIG_PATH: &str = ".agent-config.json";
    /// The roots re-imposed read-only over the worktree in use mode (INV-24,
    /// ADR 0030): the whole definition surface, including the `.pi/` dir the
    /// persona lives under and the notes file the layout reserves.
    pub const READONLY_ROOTS: &[&str] = &[".pi", INSTRUCTIONS_PATH, "CLAUDE.md", CONFIG_PATH];

    /// The neutral agent definition. Today it is materialized as the Pi-native
    /// file layout ([`AgentDefinition::seed_files`]); a future adapter
    /// materializes the same type as its own package shape (SUB-3) — same
    /// type, new materializer.
    pub struct AgentDefinition {
        /// Persona ([`SYSTEM_PATH`] content).
        pub system: String,
        /// Instructions ([`INSTRUCTIONS_PATH`] content).
        pub instructions: String,
        /// Raw config body ([`CONFIG_PATH`]), when the definition carries one.
        pub config: Option<String>,
    }

    impl AgentDefinition {
        /// The ONE layout choke point: the definition rendered as the files
        /// seeded onto a fresh workspace mainline.
        pub fn seed_files(&self) -> Vec<(String, String)> {
            let mut files = vec![
                (SYSTEM_PATH.to_string(), self.system.clone()),
                (INSTRUCTIONS_PATH.to_string(), self.instructions.clone()),
            ];
            if let Some(config) = &self.config {
                files.push((CONFIG_PATH.to_string(), config.clone()));
            }
            files
        }
    }
}

/// How the membrane treats effects not explicitly named by policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Posture {
    /// In-workspace effects proceed without prompting (allow-and-record). The
    /// MVP default.
    #[default]
    TrustByDefault,
    /// Risky effects (network, out-of-workspace writes) require approval (stage).
    PromptOnRisk,
    /// Only effects explicitly allowed by policy proceed; everything else blocks.
    PolicyOnlyBlock,
}

/// The agent's declared policy — the file-level form of resource-access + the
/// egress membrane. Unknown fields are ignored so the original richer file
/// loads cleanly.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct AgentConfig {
    /// Provider to pin (e.g. `openai-codex`). M0 defaults to the user's OAuth
    /// codex endpoint when unset (see the engine).
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// Reasoning-effort level for this chat (Pi `--thinking`: off | minimal | low |
    /// medium | high | xhigh). Unset → Pi's per-model default (LLM-1, ADR 0062).
    #[serde(default)]
    pub thinking: Option<String>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub policy: Policy,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct Policy {
    #[serde(default)]
    pub posture: Posture,
    /// Tools always allowed (in-workspace file/read tools, etc.).
    #[serde(default)]
    pub allow_tools: BTreeSet<String>,
    /// Tools always blocked, regardless of posture.
    #[serde(default)]
    pub block_tools: BTreeSet<String>,
    /// Whether network egress is permitted at all in M0 (default: no).
    #[serde(default)]
    pub allow_network: bool,
}

impl AgentConfig {
    pub fn from_json(s: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(s)
    }
    pub fn from_file(path: &std::path::Path) -> std::io::Result<Self> {
        let s = std::fs::read_to_string(path)?;
        Self::from_json(&s).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }
}

/// One effect the agent is about to perform, as classified by the plugin.
#[derive(Clone, Debug)]
pub struct Effect {
    pub tool: String,
    /// True if the effect leaves the workspace (network, or a write outside the
    /// admitted workspace root).
    pub leaves_workspace: bool,
    /// What the effect acts on (the file path / target), when the tool reports one.
    /// Used by the method-definition write-gate (`INV-24`).
    pub target: Option<String>,
}

impl Effect {
    /// An in-workspace tool effect (the common case: read/edit/write a file).
    pub fn in_workspace(tool: impl Into<String>) -> Self {
        Self {
            tool: tool.into(),
            leaves_workspace: false,
            target: None,
        }
    }
    /// An effect that crosses the boundary (network call, external write).
    pub fn external(tool: impl Into<String>) -> Self {
        Self {
            tool: tool.into(),
            leaves_workspace: true,
            target: None,
        }
    }
    /// Attach the target path the effect acts on.
    pub fn with_target(mut self, target: Option<String>) -> Self {
        self.target = target;
        self
    }
}

/// The membrane's decision for an effect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Allowed under policy — execute and record (`INV` audit trail kept).
    Allow,
    /// Blocked — emit an observation; the effect does not happen.
    Block(&'static str),
    /// Held pending an explicit resource-access / resource-export grant (A5b).
    Stage(&'static str),
}

/// The egress membrane: classifies effects against a policy. Stateless and
/// deterministic — the same effect under the same policy always decides the same.
pub struct Membrane {
    policy: Policy,
    mode: AuthoringMode,
}

impl Membrane {
    pub fn new(policy: Policy) -> Self {
        Self {
            policy,
            mode: AuthoringMode::default(),
        }
    }

    /// Set the engagement's authoring mode (drives the `INV-24` write-gate).
    pub fn with_mode(mut self, mode: AuthoringMode) -> Self {
        self.mode = mode;
        self
    }

    /// The chokepoint. Every effect is classified here before it executes.
    pub fn classify(&self, effect: &Effect) -> Decision {
        // An explicit block always wins — even trust-by-default cannot override it.
        if self.policy.block_tools.contains(&effect.tool) {
            return Decision::Block("tool blocked by policy");
        }

        // INV-24: the method-definition surface is edit-authored. A write to it
        // from a use-mode engagement is blocked even under trust-by-default — the
        // agent cannot rewrite its own system prompt or loosen its own policy.
        if self.mode != AuthoringMode::Edit && is_write_tool(&effect.tool) {
            if let Some(t) = &effect.target {
                if is_method_surface_path(t) {
                    return Decision::Block("method definition is read-only in use mode");
                }
            }
        }

        if effect.leaves_workspace {
            // Crossing the boundary needs an explicit basis; never the default.
            return if self.policy.allow_network {
                Decision::Allow
            } else {
                match self.policy.posture {
                    Posture::PromptOnRisk => Decision::Stage("external effect: needs approval"),
                    _ => Decision::Block("external effect not permitted (no network basis)"),
                }
            };
        }

        // In-workspace effects: explicit allow, or posture's default.
        if self.policy.allow_tools.contains(&effect.tool) {
            return Decision::Allow;
        }
        match self.policy.posture {
            Posture::TrustByDefault => Decision::Allow,
            Posture::PromptOnRisk => Decision::Stage("unlisted tool: confirm in-workspace effect"),
            Posture::PolicyOnlyBlock => Decision::Block("tool not in allow-list"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust_by_default_allows_in_workspace_blocks_external() {
        let m = Membrane::new(Policy::default()); // posture defaults to trust-by-default
        assert_eq!(m.classify(&Effect::in_workspace("edit")), Decision::Allow);
        // even trust-by-default does NOT let payload leave the boundary by default
        assert!(matches!(
            m.classify(&Effect::external("fetch")),
            Decision::Block(_)
        ));
    }

    #[test]
    fn explicit_block_overrides_posture() {
        let policy = Policy {
            posture: Posture::TrustByDefault,
            block_tools: ["bash"].iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
        let m = Membrane::new(policy);
        assert!(matches!(
            m.classify(&Effect::in_workspace("bash")),
            Decision::Block(_)
        ));
        assert_eq!(m.classify(&Effect::in_workspace("read")), Decision::Allow);
    }

    #[test]
    fn policy_only_block_is_an_allowlist() {
        let policy = Policy {
            posture: Posture::PolicyOnlyBlock,
            allow_tools: ["read", "edit"].iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        };
        let m = Membrane::new(policy);
        assert_eq!(m.classify(&Effect::in_workspace("read")), Decision::Allow);
        assert!(matches!(
            m.classify(&Effect::in_workspace("write")),
            Decision::Block(_)
        ));
    }

    #[test]
    fn prompt_on_risk_stages_external() {
        let policy = Policy {
            posture: Posture::PromptOnRisk,
            ..Default::default()
        };
        let m = Membrane::new(policy);
        assert!(matches!(
            m.classify(&Effect::external("fetch")),
            Decision::Stage(_)
        ));
    }

    #[test]
    fn parses_agent_config_and_ignores_unknown_fields() {
        let cfg = AgentConfig::from_json(
            r#"{
                "model": "gpt-5.5",
                "tools": ["read","edit","write"],
                "policy": { "posture": "policy-only-block", "allow_tools": ["read"] },
                "builderOnly": ["builder_only/**"]
            }"#,
        )
        .unwrap();
        assert_eq!(cfg.model.as_deref(), Some("gpt-5.5"));
        assert_eq!(cfg.policy.posture, Posture::PolicyOnlyBlock);
        assert!(cfg.policy.allow_tools.contains("read"));
    }

    // ── INV-24: method definition is edit-authored (ADR 0029) ──────────────────

    fn edit(target: &str) -> Effect {
        Effect::in_workspace("edit").with_target(Some(target.to_string()))
    }

    #[test]
    fn use_mode_blocks_writes_to_the_method_surface() {
        let m = Membrane::new(Policy::default()).with_mode(AuthoringMode::Use);
        for path in [
            ".pi/SYSTEM.md",
            "AGENTS.md",
            ".agent-config.json",
            ".pi/extensions/x.ts",
            "./AGENTS.md",
            "/abs/wt/.pi/SYSTEM.md",
        ] {
            assert!(
                matches!(m.classify(&edit(path)), Decision::Block(_)),
                "use mode must block writing {path}"
            );
        }
        // but use mode can still WRITE non-definition files (the actual work)…
        assert_eq!(m.classify(&edit("src/main.rs")), Decision::Allow);
        // …and can READ its own definition (read is not a write tool).
        let read_sys = Effect::in_workspace("read").with_target(Some(".pi/SYSTEM.md".into()));
        assert_eq!(m.classify(&read_sys), Decision::Allow);
    }

    #[test]
    fn edit_mode_may_write_the_method_surface() {
        let m = Membrane::new(Policy::default()).with_mode(AuthoringMode::Edit);
        assert_eq!(m.classify(&edit(".pi/SYSTEM.md")), Decision::Allow);
        assert_eq!(m.classify(&edit("AGENTS.md")), Decision::Allow);
        assert_eq!(m.classify(&edit(".agent-config.json")), Decision::Allow);
    }

    /// The definition layout has exactly one choke point: `seed_files` renders
    /// the neutral definition as the Pi-native layout, config included only
    /// when the definition carries one.
    #[test]
    fn seed_files_is_the_one_layout_choke_point() {
        let def = definition::AgentDefinition {
            system: "persona".into(),
            instructions: "conventions".into(),
            config: None,
        };
        assert_eq!(
            def.seed_files(),
            vec![
                (".pi/SYSTEM.md".to_string(), "persona".to_string()),
                ("AGENTS.md".to_string(), "conventions".to_string()),
            ]
        );

        let with_config = definition::AgentDefinition {
            config: Some("{}".into()),
            ..def
        };
        assert_eq!(
            with_config.seed_files().last(),
            Some(&(".agent-config.json".to_string(), "{}".to_string()))
        );
        // every seeded file sits on the read-only definition surface
        for (path, _) in with_config.seed_files() {
            assert!(is_method_surface_path(&path), "{path} is on the surface");
        }
    }

    use proptest::prelude::*;

    proptest! {
        // METHOD_WRITE_REQUIRES_EDIT (INV-24): for any tool/target/mode, an
        // *admitted* write to the method surface implies edit mode. Mirrors
        // models/method-integrity.qnt over the concrete membrane.
        #[test]
        fn method_write_requires_edit(
            tool in prop::sample::select(vec!["read", "edit", "write", "ls", "grep"]),
            is_edit in any::<bool>(),
            // a path that is sometimes on the surface, sometimes not
            surface in prop::sample::select(vec![".pi/SYSTEM.md", "AGENTS.md", ".agent-config.json",
                                                 ".pi/extensions/a.ts", "src/lib.rs", "notes/x.md", "data.csv"]),
        ) {
            let mode = if is_edit { AuthoringMode::Edit } else { AuthoringMode::Use };
            let m = Membrane::new(Policy::default()).with_mode(mode);
            let eff = Effect::in_workspace(tool).with_target(Some(surface.to_string()));
            let decision = m.classify(&eff);

            let is_surface_write = is_write_tool(tool) && is_method_surface_path(surface);
            if is_surface_write && !is_edit {
                prop_assert!(matches!(decision, Decision::Block(_)),
                    "use-mode write to surface must block: {tool} {surface}");
            }
            // The invariant: any ADMITTED (Allow) surface write was in edit mode.
            if matches!(decision, Decision::Allow) && is_write_tool(tool) && is_method_surface_path(surface) {
                prop_assert!(is_edit, "an admitted surface write must be edit mode");
            }
        }
    }
}
