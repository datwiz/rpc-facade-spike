//! Wire types shared by every participant: the client, the loopback handler
//! and the Lambda binary all speak exactly these messages.
//!
//! Nothing in here knows about transports. The only non-generated type is
//! [`Envelope`], which is the JSON wrapper AWS Lambda forces on us.

pub mod envelope;

/// Generated from `proto/something/v1/something.proto`.
pub mod v1 {
    include!(concat!(env!("OUT_DIR"), "/something.v1.rs"));
}

pub use envelope::{Envelope, EnvelopeError};

use v1::VersionRange;

/// Convenience constructor; `VersionRange` is generated so it has no helpers.
pub fn version_range(min: u32, max: u32) -> VersionRange {
    VersionRange { min, max }
}

/// Inclusive containment test used by both the server and the negotiator.
pub fn range_contains(range: &VersionRange, version: u32) -> bool {
    version >= range.min && version <= range.max
}

/// Human-readable form for error messages, e.g. `1-2`.
pub fn range_display(range: &VersionRange) -> String {
    format!("{}-{}", range.min, range.max)
}
