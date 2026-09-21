//! The client half: a facade whose callers cannot tell whether the work
//! happened in this process or in a Lambda execution environment.
//!
//! The trick is that there is only one code path. `Something` always encodes a
//! protobuf request, hands the bytes to a [`Transport`], and decodes protobuf
//! bytes back. "Local" is not a shortcut around that path — it is a transport
//! that happens to call the handler in-process.

mod error;
mod negotiate;
mod something;
mod transport;

#[cfg(feature = "aws")]
mod lambda_transport;

pub use error::{Error, TransportError};
pub use negotiate::{negotiate, ClientRequirements, Negotiated};
pub use something::{DoSomething, Mode, Something};
pub use transport::{LoopbackTransport, Transport};

#[cfg(feature = "aws")]
pub use lambda_transport::LambdaTransport;

pub use something_core::{capability, proto, Handler, HandlerConfig};
