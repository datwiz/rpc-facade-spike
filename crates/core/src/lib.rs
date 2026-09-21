//! The server half of the protocol, as a pure library.
//!
//! This crate deliberately has no Lambda, AWS, async or I/O dependencies. That
//! is what lets the *same* code answer both an in-process loopback call and a
//! real Lambda invocation, so the local and remote modes cannot drift apart.

mod handler;

pub use handler::{capability, Handler, HandlerConfig};

/// Re-exported so downstream crates need not depend on `something-proto`
/// directly just to name a capability or a version range.
pub use something_proto as proto;
