use std::time::Duration;

use aws_sdk_lambda::primitives::Blob;
use something_proto::Envelope;
use tokio::runtime::Runtime;

use crate::error::{Error, TransportError};
use crate::transport::Transport;

/// Environment variables, kept together so the README and the code agree.
const FUNCTION_VAR: &str = "SOMETHING_LAMBDA_FUNCTION";
const QUALIFIER_VAR: &str = "SOMETHING_LAMBDA_QUALIFIER";
const TIMEOUT_VAR: &str = "SOMETHING_LAMBDA_TIMEOUT_SECS";

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Invokes a Lambda function synchronously (`RequestResponse`).
///
/// ## The async trade-off
///
/// The AWS SDK is async and the facade is sync, so this type owns a small
/// current-thread tokio runtime and blocks on it. That keeps the PyO3 layer
/// trivial, at one real cost: **calling this from inside an async context
/// panics** (`block_on` inside a runtime). An async facade is the fix if this
/// ever grows past a PoC.
pub struct LambdaTransport {
    runtime: Runtime,
    client: aws_sdk_lambda::Client,
    function: String,
    /// A published version or alias. Never `$LATEST`: register and exec must
    /// hit the same code, and `$LATEST` can change between the two.
    qualifier: String,
    timeout: Duration,
}

impl LambdaTransport {
    /// Reads function name, qualifier and timeout from the environment and
    /// loads the default AWS credential chain.
    pub fn from_env() -> Result<Self, Error> {
        let function = std::env::var(FUNCTION_VAR)
            .map_err(|_| Error::Config(format!("{FUNCTION_VAR} is required in remote mode")))?;

        // Required, not defaulted: silently falling back to $LATEST is the
        // exact version-skew bug this protocol is trying to make visible.
        let qualifier = std::env::var(QUALIFIER_VAR).map_err(|_| {
            Error::Config(format!(
                "{QUALIFIER_VAR} is required: set it to a published version or alias so the \
                 register and exec calls reach the same code"
            ))
        })?;

        let timeout = match std::env::var(TIMEOUT_VAR) {
            Ok(value) => Duration::from_secs(value.parse::<u64>().map_err(|err| {
                Error::Config(format!("{TIMEOUT_VAR} must be whole seconds: {err}"))
            })?),
            Err(_) => DEFAULT_TIMEOUT,
        };

        LambdaTransport::new(function, qualifier, timeout)
    }

    pub fn new(function: String, qualifier: String, timeout: Duration) -> Result<Self, Error> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| Error::Config(format!("could not start a tokio runtime: {err}")))?;

        // Credentials and region resolve once, at construction, so the first
        // call does not pay for IMDS or SSO lookups.
        // `load_defaults` with an explicit behaviour version, so an SDK
        // upgrade cannot silently change retry/timeout defaults underneath us.
        let config = runtime.block_on(aws_config::load_defaults(
            aws_config::BehaviorVersion::latest(),
        ));
        let client = aws_sdk_lambda::Client::new(&config);

        Ok(LambdaTransport {
            runtime,
            client,
            function,
            qualifier,
            timeout,
        })
    }

    async fn invoke(&self, request: &[u8]) -> Result<Vec<u8>, TransportError> {
        let envelope = Envelope::from_bytes(request)
            .to_json()
            .map_err(|err| TransportError::new(format!("could not build envelope: {err}")))?;

        let output = self
            .client
            .invoke()
            .function_name(&self.function)
            .qualifier(&self.qualifier)
            .payload(Blob::new(envelope))
            .send()
            .await
            .map_err(|err| {
                TransportError::new(format!(
                    "invoke {}:{} failed: {}",
                    self.function,
                    self.qualifier,
                    aws_sdk_lambda::error::DisplayErrorContext(&err)
                ))
            })?;

        // A handled error response arrives as a normal payload; this field is
        // set only when the function itself failed (panic, timeout, init
        // error), which is a transport problem, not a protocol one.
        if let Some(function_error) = output.function_error() {
            let detail = output
                .payload()
                .map(|blob| String::from_utf8_lossy(blob.as_ref()).to_string())
                .unwrap_or_default();
            return Err(TransportError::new(format!(
                "lambda function error ({function_error}): {detail}"
            )));
        }

        let payload = output
            .payload()
            .ok_or_else(|| TransportError::new("lambda returned an empty payload"))?;

        Envelope::from_json(payload.as_ref())
            .and_then(|envelope| envelope.to_bytes())
            .map_err(|err| TransportError::new(format!("bad response envelope: {err}")))
    }
}

impl Transport for LambdaTransport {
    fn call(&self, request: &[u8]) -> Result<Vec<u8>, TransportError> {
        // Panics if called from within an async runtime — see the type docs.
        self.runtime.block_on(async {
            tokio::time::timeout(self.timeout, self.invoke(request))
                .await
                .map_err(|_| {
                    TransportError::new(format!(
                        "invoke {}:{} timed out after {:?}",
                        self.function, self.qualifier, self.timeout
                    ))
                })?
        })
    }

    fn describe(&self) -> String {
        format!("lambda({}:{})", self.function, self.qualifier)
    }
}
