//! The Lambda invoke API carries JSON, not bytes, so protobuf travels as
//! base64 inside a one-field JSON object.
//!
//! This envelope is the one thing that can never be renegotiated: both sides
//! must agree on it before any version handshake is possible. So it stays
//! deliberately tiny and frozen, and all evolution happens inside `payload`.

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EnvelopeError {
    #[error("envelope is not valid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("envelope payload is not valid base64: {0}")]
    Base64(#[from] base64::DecodeError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub payload: String,
}

impl Envelope {
    /// Wrap protobuf bytes for transport.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Envelope {
            payload: STANDARD.encode(bytes),
        }
    }

    /// Unwrap back to protobuf bytes.
    pub fn to_bytes(&self) -> Result<Vec<u8>, EnvelopeError> {
        Ok(STANDARD.decode(self.payload.as_bytes())?)
    }

    pub fn to_json(&self) -> Result<Vec<u8>, EnvelopeError> {
        Ok(serde_json::to_vec(self)?)
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, EnvelopeError> {
        Ok(serde_json::from_slice(bytes)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_json() {
        let bytes = vec![0x08, 0x96, 0x01, 0xff, 0x00];
        let json = Envelope::from_bytes(&bytes).to_json().unwrap();
        let back = Envelope::from_json(&json).unwrap().to_bytes().unwrap();
        assert_eq!(bytes, back);
    }

    #[test]
    fn round_trips_empty_payload() {
        let json = Envelope::from_bytes(&[]).to_json().unwrap();
        assert!(Envelope::from_json(&json)
            .unwrap()
            .to_bytes()
            .unwrap()
            .is_empty());
    }

    #[test]
    fn rejects_non_json() {
        assert!(matches!(
            Envelope::from_json(b"not json"),
            Err(EnvelopeError::Json(_))
        ));
    }

    #[test]
    fn rejects_missing_payload_field() {
        assert!(matches!(
            Envelope::from_json(br#"{"data":"aGk="}"#),
            Err(EnvelopeError::Json(_))
        ));
    }

    #[test]
    fn rejects_bad_base64() {
        let env = Envelope::from_json(br#"{"payload":"!!! not base64 !!!"}"#).unwrap();
        assert!(matches!(env.to_bytes(), Err(EnvelopeError::Base64(_))));
    }
}
