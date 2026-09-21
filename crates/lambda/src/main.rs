//! The Lambda entry point. It owns nothing but the adapter: JSON envelope in,
//! bytes to `something-core`, JSON envelope out.
//!
//! The binary is named `bootstrap` because that is what the `provided.al2023`
//! runtime executes.

use lambda_runtime::{service_fn, Error, LambdaEvent};
use something_core::{Handler, HandlerConfig};
use something_proto::Envelope;

#[tokio::main]
async fn main() -> Result<(), Error> {
    // `main` runs once per execution environment (a cold start), and the
    // handler closure runs once per invocation. Anything expensive belongs
    // here, above the closure, so warm invocations do not repeat it.
    let handler = Handler::new(HandlerConfig::new(build_id()));

    lambda_runtime::run(service_fn(|event: LambdaEvent<Envelope>| {
        let handler = &handler;
        async move { invoke(handler, event.payload) }
    }))
    .await
}

/// Identifies the deployed code to clients. AWS sets these; the fallback keeps
/// local `cargo lambda watch` runs identifiable.
fn build_id() -> String {
    let name = std::env::var("AWS_LAMBDA_FUNCTION_NAME").unwrap_or_else(|_| "local".to_string());
    let version =
        std::env::var("AWS_LAMBDA_FUNCTION_VERSION").unwrap_or_else(|_| "dev".to_string());
    format!("{name}:{version}")
}

/// Kept sync and separate from the runtime wiring so it stays obvious that no
/// protocol decision is made in the adapter.
fn invoke(handler: &Handler, envelope: Envelope) -> Result<Envelope, Error> {
    // A bad envelope is the one failure this layer cannot express in-protocol:
    // without decodable bytes there is no request to answer. It becomes a
    // Lambda function error, which the client reports as a transport failure.
    let request = envelope.to_bytes()?;

    Ok(Envelope::from_bytes(&handler.handle_bytes(&request)))
}
