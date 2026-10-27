//! The HTTP interface: routes, handlers and their request and response bodies.

pub mod dto;
mod handlers;

use std::sync::Arc;

use axum::{
    http::{header, HeaderValue},
    routing::{get, post},
    Router,
};
use tower_http::{services::ServeDir, set_header::SetResponseHeader, trace::TraceLayer};

use crate::application::NodeService;

#[derive(Clone)]
pub struct AppState {
    pub node: Arc<NodeService>,
}

impl AppState {
    pub fn new(node: NodeService) -> Self {
        Self { node: Arc::new(node) }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/chain", get(handlers::get_chain).post(handlers::replace_chain))
        .route("/chain/valid", get(handlers::get_validity))
        .route("/wallets", post(handlers::create_wallet))
        .route("/balances/:address", get(handlers::get_balance))
        .route("/transactions", post(handlers::submit_transaction))
        .route("/transactions/sign", post(handlers::sign_transaction))
        .route("/transactions/pending", get(handlers::get_pending))
        .route("/mine", post(handlers::mine))
        .fallback_service(web_ui())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// The files in `public/`. Browsers must revalidate them on every load, so the page, its
/// styles and its script always come from the same version.
fn web_ui() -> SetResponseHeader<ServeDir, HeaderValue> {
    SetResponseHeader::overriding(
        ServeDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/public")),
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-cache"),
    )
}
