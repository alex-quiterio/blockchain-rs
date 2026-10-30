//! Which handler answers each path, grouped as the handlers are, and the web UI for everything
//! else.

use axum::{
    http::{header, HeaderValue},
    routing::{get, post},
    Router,
};
use tower_http::{services::ServeDir, set_header::SetResponseHeader, trace::TraceLayer};

use super::handlers::{blocks, chain, network, transactions, wallets};
use super::AppState;

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/chain", get(chain::get_chain).post(chain::replace_chain))
        .route("/chain/valid", get(chain::get_validity))
        .route("/chain/work", get(chain::get_work))
        .route("/status", get(chain::get_status))
        .route("/headers", get(chain::get_headers))
        .route("/blocks/:id", get(chain::get_block))
        .route("/wallets", post(wallets::create_wallet))
        .route("/balances/:address", get(wallets::get_balance))
        .route("/transactions", post(transactions::submit_transaction))
        .route("/transactions/sign", post(transactions::sign_transaction))
        .route("/transactions/pending", get(transactions::get_pending))
        .route("/transactions/:signature", get(transactions::get_transaction))
        .route("/transactions/:signature/proof", get(transactions::get_proof))
        .route("/addresses/:address/transactions", get(transactions::get_address_transactions))
        .route("/fees", get(transactions::get_fees))
        .route("/mine", post(blocks::mine))
        .route("/forge", post(blocks::forge))
        .route("/forge/proposal", post(blocks::propose))
        .route("/blocks", post(blocks::submit_block))
        .route("/consensus", get(blocks::get_consensus))
        .route("/peers", get(network::get_peers).post(network::add_peer))
        .route("/sync", post(network::sync))
        .route("/node", get(network::get_node))
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
