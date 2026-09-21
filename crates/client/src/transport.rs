use prost::Message;
use something_core::{Handler, HandlerConfig};

use crate::error::TransportError;

/// Bytes in, bytes out. Everything above this line is protocol; everything
/// below it is plumbing.
///
/// Deliberately synchronous: the facade is sync (see the README), so an async
/// trait here would only push `block_on` somewhere less obvious.
pub trait Transport: Send + Sync {
    /// `request` and the returned bytes are both encoded `something.v1`
    /// messages. Framing (the Lambda JSON envelope, say) is the transport's
    /// private business.
    fn call(&self, request: &[u8]) -> Result<Vec<u8>, TransportError>;

    /// For diagnostics and the README's "which mode am I in?" question.
    fn describe(&self) -> String;
}

/// Runs the handler in this process — but through the full encode/decode path.
///
/// Short-circuiting the serialization here would be faster and would defeat
/// the purpose: local mode exists to catch protocol mistakes before they reach
/// the Lambda, so it has to be honest about versioning, framing and errors.
pub struct LoopbackTransport {
    handler: Handler,
}

impl LoopbackTransport {
    pub fn new(handler: Handler) -> Self {
        LoopbackTransport { handler }
    }

    pub fn with_config(config: HandlerConfig) -> Self {
        LoopbackTransport::new(Handler::new(config))
    }

    pub fn handler(&self) -> &Handler {
        &self.handler
    }
}

impl Default for LoopbackTransport {
    fn default() -> Self {
        LoopbackTransport::new(Handler::default())
    }
}

impl Transport for LoopbackTransport {
    fn call(&self, request: &[u8]) -> Result<Vec<u8>, TransportError> {
        // The handler is infallible, so loopback can never produce a transport
        // error. That asymmetry is real: in-process calls do not time out.
        Ok(self.handler.handle_bytes(request))
    }

    fn describe(&self) -> String {
        format!("loopback({})", self.handler.config().build)
    }
}

/// Round-trip a typed request through a transport.
pub(crate) fn round_trip(
    transport: &dyn Transport,
    request: something_proto::v1::Request,
) -> Result<something_proto::v1::Response, crate::Error> {
    let bytes = transport.call(&request.encode_to_vec())?;
    Ok(something_proto::v1::Response::decode(bytes.as_slice())?)
}
