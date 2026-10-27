use std::sync::{Arc, Mutex};

use axum::{
    routing::{get, post},
    Router,
};
use tower_http::{services::ServeDir, trace::TraceLayer};

use crate::api;
use crate::chain::Blockchain;

#[derive(Clone)]
pub struct AppState {
    pub chain: Arc<Mutex<Blockchain>>,
}

impl AppState {
    pub fn new(chain: Blockchain) -> Self {
        Self { chain: Arc::new(Mutex::new(chain)) }
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/chain", get(api::get_chain))
        .route("/chain/valid", get(api::get_validity))
        .route("/wallets", post(api::create_wallet))
        .route("/balances/:address", get(api::get_balance))
        .route("/transactions", post(api::submit_transaction))
        .route("/transactions/sign", post(api::sign_transaction))
        .route("/transactions/pending", get(api::get_pending))
        .route("/mine", post(api::mine))
        .fallback_service(ServeDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/public")))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
