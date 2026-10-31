//! How the domain's refusals and the node's failures reach a client: an HTTP status and a JSON
//! body saying why.

use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};

use crate::api::dto::error::ErrorResponse;
use crate::application::ApplicationError;
use crate::application::NodeError;
use crate::domain::accounting::TransactionError;
use crate::domain::chain::block::BlockError;
use crate::domain::chain::ChainError;
use crate::domain::crypto::ValueError;

/// Maps domain and application errors onto HTTP status codes and JSON error bodies.
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    pub(super) fn bad_request(message: impl Into<String>) -> Self {
        Self { status: StatusCode::BAD_REQUEST, message: message.into() }
    }

    pub(super) fn not_found(message: impl Into<String>) -> Self {
        Self { status: StatusCode::NOT_FOUND, message: message.into() }
    }

    pub(super) fn forbidden(message: impl Into<String>) -> Self {
        Self { status: StatusCode::FORBIDDEN, message: message.into() }
    }
}

impl From<ApplicationError> for ApiError {
    fn from(err: ApplicationError) -> Self {
        let status = match &err {
            ApplicationError::Value(_) => StatusCode::BAD_REQUEST,
            ApplicationError::Transaction(TransactionError::BadSignature) => {
                StatusCode::UNAUTHORIZED
            }
            ApplicationError::Transaction(
                TransactionError::ZeroAmount
                | TransactionError::UnexpectedCoinbase
                | TransactionError::AlreadyPending,
            ) => StatusCode::BAD_REQUEST,
            ApplicationError::Transaction(_) => StatusCode::UNPROCESSABLE_ENTITY,
            ApplicationError::Block(BlockError::NotYourTurn { .. }) => StatusCode::FORBIDDEN,
            ApplicationError::Block(
                BlockError::NotProofOfWork | BlockError::NotProofOfStake | BlockError::StaleBlock,
            ) => StatusCode::CONFLICT,
            ApplicationError::Block(_) => StatusCode::BAD_REQUEST,
            ApplicationError::Chain(ChainError::ReorgTooDeep { .. }) => StatusCode::CONFLICT,
            ApplicationError::Chain(ChainError::Empty | ChainError::InvalidBlock { .. }) => {
                StatusCode::BAD_REQUEST
            }
        };
        Self { status, message: err.to_string() }
    }
}

/// Each kind of refusal maps the same way, through `ApplicationError`.
macro_rules! via_application_error {
    ($($kind:ty),*) => {$(
        impl From<$kind> for ApiError {
            fn from(err: $kind) -> Self {
                ApplicationError::from(err).into()
            }
        }
    )*};
}

via_application_error!(ValueError, TransactionError, BlockError, ChainError);

impl From<NodeError> for ApiError {
    fn from(err: NodeError) -> Self {
        match err {
            NodeError::Domain(e) => e.into(),
            NodeError::Storage(_) => {
                Self { status: StatusCode::INTERNAL_SERVER_ERROR, message: err.to_string() }
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(ErrorResponse { message: self.message })).into_response()
    }
}
