//! gaugewright local control plane — the axum HTTP surface over the admission shell.
//!
//! Clients submit commands and query projections; the server never owns truth
//! beyond the event log (`INV-5`). One co-resident server backs desktop, web,
//! and (later) remote (`app-stack.md`). The per-process mutex on the
//! [`Workbench`] serializes admission (single-writer per scope, `INV-7`).
//!
//! The control plane exposes the run lifecycle plus the engagement surface:
//! create a worktree off the instance `main`, query its run state, and read the
//! reviewer's diff. This is the thin API the Solid frontend develops against.

pub mod account;
pub mod account_routes;
pub mod advancement;
pub mod app_support;
pub mod at_rest;
pub mod attention;
pub mod attestation_verifier;
pub mod audit;
pub mod boundary_keeper;
pub mod challenge;
pub mod codex_oauth;
pub mod content_vault;
pub mod crypto_erasure;
pub mod device_enroll;
pub mod device_enroll_drive;
pub mod directory_sync;
pub mod engagement_routes;
pub mod engine;
pub mod facility;
pub mod facility_routes;
pub mod fed_harness;
pub mod federation;
pub mod federation_relay;
pub mod harness_select;
pub mod identity;
pub mod key_store;
pub mod library;
pub mod library_routes;
pub mod library_state;
pub mod lifecycle_routes;
pub mod local_routes;
pub mod measurement_store;
pub mod mobile_bridge;
pub mod net_http;
pub mod net_relay;
pub mod net_server;
pub mod net_tls;
pub mod onboarding;
pub mod open_api;
pub mod open_route_stack;
pub mod open_runtime;
pub mod org;
pub mod package_flow;
pub mod package_routes;
pub mod package_store;
pub mod policy_compiler;
pub mod project_credential_routes;
pub mod remote_runtime;
pub mod resource_store;
pub mod secret;
pub mod session;
pub mod session_activity;
pub mod stream;
pub mod tenancy;
pub mod throttle;
pub mod workbench_auth;
pub mod workbench_state;
pub mod workstream_routes;
pub(crate) use app_support::io;
pub use app_support::LockUnpoisoned;
pub use app_support::{
    AttestationMode, DEFAULT_AGENT, DEFAULT_INSTANCE, DEFAULT_PLACEMENT, DEFAULT_PROJECT,
    LOCAL_AUTHORITY,
};
pub use gaugewright_whip_runtime::{AdmittedPolicyEpoch, PolicyAdmissionError, PolicyEpoch};
pub use open_route_stack::open_control_plane;
pub use open_runtime::{open_control_plane_root, open_serve};
pub(crate) use workbench_state::build_workbench;
pub use workbench_state::{
    open_workbench, open_workbench_with_content_keywrap, SharedWorkbench, Workbench,
};

#[cfg(test)]
pub(crate) mod test_support;

pub(crate) use net_http::err_response;
pub(crate) use stream::ServerEvent;

#[cfg(test)]
mod tests;
