// Copyright (c) 2024 Tencent Inc.
// SPDX-License-Identifier: Apache-2.0
//

use axum::{
    middleware,
    routing::{delete, get, patch, post},
    Router,
};
use std::time::Duration;
use tower::ServiceBuilder;
use tower_http::{
    compression::CompressionLayer,
    cors::CorsLayer,
    request_id::{MakeRequestUuid, SetRequestIdLayer},
    timeout::TimeoutLayer,
    trace::TraceLayer,
};

use crate::{
    handlers::{health, sandboxes, templates},
    middleware::{auth::unified_auth, rate_limit::rate_limit},
    state::AppState,
};

pub fn build_router(state: AppState) -> Router {
    let auth_configured = state.config.auth_callback_url.as_deref().is_some_and(|u| !u.is_empty());

    // ── Sandbox routes ────────────────────────────────────────────────────
    let sandbox_routes = Router::new()
        .route("/sandboxes", get(sandboxes::list_sandboxes))
        .route("/sandboxes", post(sandboxes::create_sandbox))
        .route("/v2/sandboxes", get(sandboxes::list_sandboxes_v2))
        .route("/sandboxes/:sandboxID", get(sandboxes::get_sandbox))
        .route("/sandboxes/:sandboxID", delete(sandboxes::kill_sandbox))
        .route("/sandboxes/:sandboxID/logs", get(sandboxes::get_sandbox_logs))
        .route("/v2/sandboxes/:sandboxID/logs", get(sandboxes::get_sandbox_logs_v2))
        .route("/sandboxes/:sandboxID/timeout", post(sandboxes::set_sandbox_timeout))
        .route("/sandboxes/:sandboxID/refreshes", post(sandboxes::refresh_sandbox))
        .route("/sandboxes/:sandboxID/pause", post(sandboxes::pause_sandbox))
        .route("/sandboxes/:sandboxID/resume", post(sandboxes::resume_sandbox))
        .route("/sandboxes/:sandboxID/connect", post(sandboxes::connect_sandbox))
        .route("/sandboxes/:sandboxID/snapshots", post(sandboxes::create_snapshot))
        .route("/sandboxes/snapshots/:snapshotID", delete(sandboxes::delete_snapshot));

    // Conditionally attach rate-limit + unified auth
    let sandbox_routes = if auth_configured {
        sandbox_routes
            .layer(middleware::from_fn_with_state(state.clone(), rate_limit))
            .layer(middleware::from_fn_with_state(state.clone(), unified_auth))
    } else {
        sandbox_routes
    };

    // ── Template routes ───────────────────────────────────────────────────
    let template_routes = Router::new()
        .route("/templates", get(templates::list_templates))
        .route("/templates", post(templates::create_template))
        .route("/templates/:templateID", get(templates::get_template))
        .route("/templates/:templateID", post(templates::rebuild_template))
        .route("/templates/:templateID", delete(templates::delete_template))
        .route("/templates/:templateID", patch(templates::update_template))
        .route("/templates/:templateID/builds/:buildID", post(templates::start_template_build))
        .route("/templates/:templateID/builds/:buildID/status", get(templates::get_template_build_status))
        .route("/templates/:templateID/builds/:buildID/logs", get(templates::get_template_build_logs));

    // Conditionally attach unified auth
    let template_routes = if auth_configured {
        template_routes.layer(middleware::from_fn_with_state(state.clone(), unified_auth))
    } else {
        template_routes
    };

    // ── Full router with global middleware ────────────────────────────────
    Router::new()
        .route("/health", get(health::health))
        .merge(sandbox_routes)
        .merge(template_routes)
        .with_state(state)
        .layer(
            ServiceBuilder::new()
                .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
                .layer(TraceLayer::new_for_http())
                .layer(TimeoutLayer::new(Duration::from_secs(30)))
                .layer(CompressionLayer::new())
                .layer(CorsLayer::permissive()),
        )
}
