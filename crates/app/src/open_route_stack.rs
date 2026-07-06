//! Open-source control-plane route composition.

use axum::Router;

use crate::{account_routes, local_routes, net_http, LockUnpoisoned, SharedWorkbench};

pub fn open_control_plane(wb: SharedWorkbench) -> Router {
    let federation_on = {
        let g = wb.lock_unpoisoned();
        g.is_federation_enabled()
    };
    Router::new()
        .merge(local_routes::routes(federation_on))
        .merge(account_routes::routes())
        .layer(net_http::cors_layer())
        .with_state(wb)
        .layer(axum::middleware::from_fn(net_http::security_headers))
}
