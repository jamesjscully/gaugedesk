use axum::{
    routing::{delete, get, post, put},
    Router,
};

use crate::{
    engagement_routes as er, federation, library_routes as lr, lifecycle_routes as life, net_http,
    package_routes as pkg, project_credential_routes, resource_store as rs,
    workstream_routes as wr, SharedWorkbench,
};

/// Open-source local workbench route surface: health, workspace/library,
/// project/chat/resource lifecycles, package primitives, projections, test reset
/// hooks, and the parked self-operated federation route surface when its open
/// operator gate is enabled.
pub fn routes(federation_on: bool) -> Router<SharedWorkbench> {
    Router::new()
        .route("/health", get(net_http::health))
        .route("/workspace", get(lr::get_workspace))
        .route("/workspace/events", get(er::workspace_events))
        .route("/tasks", get(lr::get_tasks))
        .route("/search", get(lr::search))
        .route("/archetypes", post(lr::create_agent))
        .route(
            "/archetypes/:id",
            get(lr::get_agent)
                .put(lr::update_agent)
                .delete(lr::delete_agent),
        )
        .route("/archetypes/:id/chats", post(lr::create_chat_under_agent))
        .route("/archetypes/:id/use", post(lr::use_archetype))
        .route("/archetypes/:id/fork", post(lr::fork_archetype))
        .route(
            "/archetypes/:id/pull-from-source",
            post(lr::post_pull_from_source),
        )
        .route("/archetypes/:id/publish", post(lr::post_publish_archetype))
        .route("/placements/:id/upgrade", post(lr::post_upgrade_placement))
        .route("/placements/:id/accept", post(lr::post_accept_placement))
        .route("/projects", post(lr::create_project))
        .route(
            "/projects/:id",
            put(lr::update_project).delete(lr::delete_project),
        )
        .route("/projects/:id/home", get(lr::project_home))
        .route(
            "/projects/:id/credentials",
            get(project_credential_routes::get_project_credentials)
                .post(project_credential_routes::post_project_credential),
        )
        .route(
            "/projects/:id/credentials/:provider",
            delete(project_credential_routes::delete_project_credential),
        )
        .route("/projects/:pid/placements", post(lr::bind_agent))
        .route("/projects/:pid/placements/:iid", delete(lr::unbind_agent))
        .route(
            "/placements/:iid/workstreams",
            post(wr::create_workstream).get(wr::list_workstreams),
        )
        .route("/workstreams/:id/join", post(wr::join_workstream))
        .route("/workstreams/:id/leave", post(wr::leave_workstream))
        .route("/workstreams/:id/archive", post(wr::archive_workstream))
        .route("/workstreams/:id/promote", post(wr::promote_workstream))
        .route(
            "/projects/:pid/placements/:iid/chats",
            post(lr::create_chat_under_instance),
        )
        .route("/placements/:id", get(life::get_instance))
        .route("/placements/:id/command", post(life::post_instance_command))
        .route("/chats/:id/boundary", get(life::get_boundary))
        .route(
            "/boundaries/:bid/challenge",
            post(lr::issue_boundary_challenge),
        )
        .route("/boundaries/:bid/accept", post(lr::accept_boundary))
        .route("/pairing-requests", post(lr::create_pairing_request))
        .route("/pairing-status/:id", get(lr::get_pairing_status))
        .merge(federation::featured_routes(federation_on))
        .route("/chats/:id/fork", post(lr::fork_chat))
        .route("/chats/:id/sync", post(er::post_sync))
        .route("/chats/:id/stop", post(er::post_stop))
        .route("/chats/:id", delete(lr::delete_chat))
        .route("/chats/:id/title", put(lr::rename_chat))
        .route(
            "/chats",
            post(er::create_engagement).get(er::list_engagements),
        )
        .route("/fork-tree", get(life::get_fork_tree))
        .route("/chats/:id/diff", get(er::engagement_diff))
        .route("/chats/:id/tree", get(er::get_tree))
        .route("/chats/:id/file", get(er::get_file).put(er::put_file))
        .route("/chats/:id/merge-preview", post(er::post_merge_preview))
        .route("/chats/:id/transcript", get(er::get_transcript))
        .route("/chats/:id/audit", get(er::get_audit))
        .route("/chats/:id/events", get(er::engagement_events))
        .route("/chats/:id/task", post(er::post_task))
        .route("/chats/:id/merge", get(er::get_merge))
        .route("/chats/:id/merge/command", post(er::post_merge_command))
        .route("/chats/:id/revert", post(er::post_revert))
        .route("/chats/:id/config", get(er::get_config).put(er::put_config))
        .route("/chats/:id/context", post(rs::post_context))
        .route("/chats/:id/context/upload", post(rs::post_context_upload))
        .route("/chats/:id/resources", get(rs::get_resources))
        .route(
            "/chats/:id/resources/:rid/content",
            get(rs::get_resource_content),
        )
        .route(
            "/chats/:id/resources/:rid/tombstone",
            post(rs::post_resource_tombstone),
        )
        .route(
            "/chats/:id/resources/:rid/export",
            post(rs::post_resource_export),
        )
        .route(
            "/chats/:id/resources/:rid/export-to-disk",
            post(rs::post_resource_export_to_disk),
        )
        .route(
            "/chats/:id/resources/:rid/review",
            post(rs::post_resource_review),
        )
        .route(
            "/chats/:id/resources/:rid/access",
            get(rs::get_resource_access),
        )
        .route(
            "/chats/:id/resources/:rid/access/request",
            post(rs::post_resource_access_request),
        )
        .route(
            "/chats/:id/resources/:rid/access/approve",
            post(rs::post_resource_access_approve),
        )
        .route(
            "/chats/:id/resources/:rid/access/revoke",
            post(rs::post_resource_access_revoke),
        )
        .route(
            "/packages",
            post(pkg::post_package_publish).get(pkg::get_packages),
        )
        .route("/packages/:id/withdraw", post(pkg::post_package_withdraw))
        .route("/packages/:id/install", post(pkg::post_package_install))
        .route("/packages/:id/entitle", post(pkg::post_package_entitle))
        .route("/packages/:id/readiness", get(pkg::get_package_readiness))
        .route("/scopes/:scope/run", get(life::get_run))
        .route("/scopes/:scope/run/command", post(life::post_run_command))
        .route("/scopes/:scope/review", get(life::get_review))
        .route(
            "/scopes/:scope/review/command",
            post(life::post_review_command),
        )
        .route("/scopes/:scope/export", get(life::get_export))
        .route(
            "/scopes/:scope/export/command",
            post(life::post_export_command),
        )
        .route("/scopes/:scope/audit", get(life::get_audit))
        .route("/projections/:scope/:kind", get(life::get_projection))
        .route("/test/reset", post(er::post_test_reset))
        .route("/test/force-conflict", post(er::post_test_force_conflict))
}
