use something_proto::v1::ErrorCode;
use thiserror::Error;

/// A transport failed to deliver bytes and bring bytes back.
///
/// This is distinct from a server *error response*: a Lambda that returns an
/// `ErrorResponse` completed successfully as far as AWS is concerned. Crashes,
/// timeouts and throttling land here; protocol-level refusals land in
/// [`Error::Server`].
#[derive(Debug, Error)]
#[error("transport failure: {message}")]
pub struct TransportError {
    pub message: String,
}

impl TransportError {
    pub fn new(message: impl Into<String>) -> Self {
        TransportError {
            message: message.into(),
        }
    }
}

#[derive(Debug, Error)]
pub enum Error {
    /// The call never completed. Retrying may help; the client does not.
    #[error(transparent)]
    Transport(#[from] TransportError),

    /// Negotiation found no version both sides can speak, or a required
    /// capability is absent. The message names the actual ranges.
    #[error("incompatible: {0}")]
    Incompatible(String),

    /// The server understood the request and refused it.
    #[error("server error {code:?}: {message}")]
    Server { code: ErrorCode, message: String },

    /// A well-formed response that makes no sense here, e.g. a register
    /// response to an exec call, or an empty body.
    #[error("protocol violation: {0}")]
    Protocol(String),

    /// The bytes that came back are not a `Response`.
    #[error("could not decode response: {0}")]
    Decode(#[from] prost::DecodeError),

    /// Bad configuration, e.g. an unknown `SOMETHING_MODE`.
    #[error("configuration error: {0}")]
    Config(String),
}

impl Error {
    /// True when the server rejected the version we sent, which is the one
    /// case the client handles itself (re-register once, then retry once).
    pub(crate) fn is_unsupported_version(&self) -> bool {
        matches!(
            self,
            Error::Server {
                code: ErrorCode::UnsupportedVersion,
                ..
            }
        )
    }
}
