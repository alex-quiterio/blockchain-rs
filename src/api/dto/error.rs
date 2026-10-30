//! The body every refusal comes back in.

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ErrorResponse {
    pub message: String,
}
