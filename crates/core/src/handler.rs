use std::collections::HashMap;

use prost::Message;
use something_proto::v1::{
    exec_request, exec_response, request, response, Capability, ErrorCode, ErrorResponse,
    ExecResponse, PingResponse, RegisterResponse, Request, Response, VersionRange,
};
use something_proto::{range_contains, range_display, version_range};

/// What this server advertises about itself.
///
/// Tests build these with unusual ranges to exercise negotiation; production
/// uses [`HandlerConfig::default`].
#[derive(Debug, Clone)]
pub struct HandlerConfig {
    /// Opaque build identifier, echoed to clients for diagnostics.
    pub build: String,
    /// Request versions this server accepts.
    pub inbound: VersionRange,
    /// Response versions this server can emit.
    pub outbound: VersionRange,
    pub capabilities: Vec<Capability>,
    /// Informational only (e.g. max payload bytes); never enforced by clients.
    pub limits: HashMap<String, String>,
}

impl HandlerConfig {
    /// The PoC server: protocol 1..=1, one capability.
    pub fn new(build: impl Into<String>) -> Self {
        let mut limits = HashMap::new();
        // Lambda's synchronous invoke ceiling. Advertised so a client can fail
        // fast on oversized inputs instead of discovering it as a 413.
        limits.insert("max_payload_bytes".to_string(), "6291456".to_string());

        HandlerConfig {
            build: build.into(),
            inbound: version_range(1, 1),
            outbound: version_range(1, 1),
            capabilities: vec![capability("ping", 1, 1)],
            limits,
        }
    }

    /// Both protocol ranges at once; the PoC never needs them to differ.
    ///
    /// Deliberately does *not* touch capability ranges. A capability is
    /// available for the versions it says it is, and widening the protocol
    /// should not silently claim a capability works at a version nobody has
    /// implemented it for.
    pub fn with_protocol_range(mut self, min: u32, max: u32) -> Self {
        self.inbound = version_range(min, max);
        self.outbound = version_range(min, max);
        self
    }

    pub fn with_capabilities(mut self, capabilities: Vec<Capability>) -> Self {
        self.capabilities = capabilities;
        self
    }

    fn capability(&self, name: &str) -> Option<&Capability> {
        self.capabilities.iter().find(|c| c.name == name)
    }
}

impl Default for HandlerConfig {
    fn default() -> Self {
        HandlerConfig::new(concat!("something-core/", env!("CARGO_PKG_VERSION")))
    }
}

/// Helper for building capability entries in configs and tests.
pub fn capability(name: &str, min_version: u32, max_version: u32) -> Capability {
    Capability {
        name: name.to_string(),
        min_version,
        max_version,
    }
}

#[derive(Debug, Clone, Default)]
pub struct Handler {
    config: HandlerConfig,
}

impl Handler {
    pub fn new(config: HandlerConfig) -> Self {
        Handler { config }
    }

    pub fn config(&self) -> &HandlerConfig {
        &self.config
    }

    /// Bytes in, bytes out — the whole server contract.
    ///
    /// Infallible by construction: every failure, including undecodable input,
    /// becomes a `Response::error`. A handler that can return `Err` would push
    /// the decision of "protocol error or transport error?" out to each
    /// adapter, and they would answer it differently.
    pub fn handle_bytes(&self, request: &[u8]) -> Vec<u8> {
        let response = match Request::decode(request) {
            Ok(request) => self.handle(request),
            Err(err) => error(
                ErrorCode::InvalidRequest,
                format!("undecodable request: {err}"),
            ),
        };

        // Encoding a Response we built ourselves cannot fail.
        response.encode_to_vec()
    }

    pub fn handle(&self, request: Request) -> Response {
        match request.body {
            Some(request::Body::Register(_)) => self.register(),
            Some(request::Body::Exec(exec)) => self.exec(exec),
            None => error(
                ErrorCode::InvalidRequest,
                "request has no body; expected register or exec",
            ),
        }
    }

    fn register(&self) -> Response {
        Response {
            body: Some(response::Body::Register(RegisterResponse {
                build: self.config.build.clone(),
                capabilities: self.config.capabilities.clone(),
                inbound: Some(self.config.inbound),
                outbound: Some(self.config.outbound),
                limits: self.config.limits.clone(),
            })),
        }
    }

    /// Exec re-checks the version on every call. Registration is only a
    /// snapshot: a redeploy, or a different execution environment answering,
    /// can invalidate it between the handshake and the call.
    fn exec(&self, exec: something_proto::v1::ExecRequest) -> Response {
        let version = exec.negotiated_version;
        if version == 0 {
            return error(
                ErrorCode::MissingVersion,
                "exec requires negotiated_version; 0 means the client skipped negotiation",
            );
        }

        if !range_contains(&self.config.inbound, version)
            || !range_contains(&self.config.outbound, version)
        {
            return error(
                ErrorCode::UnsupportedVersion,
                format!(
                    "version {version} unsupported: server inbound {}, outbound {}",
                    range_display(&self.config.inbound),
                    range_display(&self.config.outbound)
                ),
            );
        }

        match exec.call {
            Some(exec_request::Call::Ping(_)) => self.ping(version),
            None => error(ErrorCode::InvalidRequest, "exec has no call; expected ping"),
        }
    }

    fn ping(&self, version: u32) -> Response {
        if let Err(response) = self.check_capability("ping", version) {
            return response;
        }

        Response {
            body: Some(response::Body::Exec(ExecResponse {
                output: Some(exec_response::Output::Ping(PingResponse {
                    message: "pong".to_string(),
                })),
            })),
        }
    }

    /// A capability's range is the set of protocol versions it is available
    /// for, so a server can retire one capability without retiring the
    /// protocol version it lived in.
    fn check_capability(&self, name: &str, version: u32) -> Result<(), Response> {
        let Some(capability) = self.config.capability(name) else {
            return Err(error(
                ErrorCode::UnsupportedCapability,
                format!("capability {name} is not available on this server"),
            ));
        };

        if version < capability.min_version || version > capability.max_version {
            return Err(error(
                ErrorCode::UnsupportedVersion,
                format!(
                    "capability {name} requires version {}-{}, got {version}",
                    capability.min_version, capability.max_version
                ),
            ));
        }

        Ok(())
    }
}

fn error(code: ErrorCode, message: impl Into<String>) -> Response {
    Response {
        body: Some(response::Body::Error(ErrorResponse {
            code: code as i32,
            message: message.into(),
        })),
    }
}
