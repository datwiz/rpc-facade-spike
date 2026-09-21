use std::str::FromStr;
use std::sync::Mutex;

use something_proto::v1::{
    exec_request, exec_response, request, response, ExecRequest, PingRequest, RegisterRequest,
    Request, Response,
};

use crate::error::Error;
use crate::negotiate::{negotiate, ClientRequirements, Negotiated};
use crate::transport::{round_trip, LoopbackTransport, Transport};

/// The operations a caller can perform. This is the whole point of the PoC:
/// nothing in this trait mentions where the work happens.
pub trait DoSomething {
    fn ping(&self) -> Result<String, Error>;
}

/// Which transport to build. Chosen once, at construction — not per call, so a
/// process cannot drift between modes mid-run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Local,
    Remote,
}

impl Mode {
    /// Reads `SOMETHING_MODE`, defaulting to `local`.
    pub fn from_env() -> Result<Mode, Error> {
        match std::env::var("SOMETHING_MODE") {
            Ok(value) => value.parse(),
            Err(std::env::VarError::NotPresent) => Ok(Mode::Local),
            Err(err) => Err(Error::Config(format!(
                "SOMETHING_MODE is unreadable: {err}"
            ))),
        }
    }
}

impl FromStr for Mode {
    type Err = Error;

    /// An unknown value is an error, never a silent fall back to local: a
    /// typo in a deployment variable must not quietly stop using the Lambda.
    fn from_str(value: &str) -> Result<Mode, Error> {
        match value.trim().to_ascii_lowercase().as_str() {
            "local" => Ok(Mode::Local),
            "remote" => Ok(Mode::Remote),
            other => Err(Error::Config(format!(
                "unknown mode {other:?}; expected \"local\" or \"remote\""
            ))),
        }
    }
}

/// The facade. Holds the transport and the cached handshake.
pub struct Something {
    transport: Box<dyn Transport>,
    requirements: ClientRequirements,
    // Interior mutability because renegotiation happens during `&self` calls.
    // A std Mutex is enough: the critical sections never await and never
    // perform I/O while held.
    negotiated: Mutex<Negotiated>,
}

impl Something {
    /// Build from `SOMETHING_MODE` (`local` by default).
    pub fn from_env() -> Result<Self, Error> {
        Something::with_mode(Mode::from_env()?)
    }

    /// Explicit mode, for tests and for the Python `mode=` argument.
    pub fn with_mode(mode: Mode) -> Result<Self, Error> {
        match mode {
            Mode::Local => Something::connect(
                Box::new(LoopbackTransport::default()),
                ClientRequirements::default(),
            ),
            Mode::Remote => Something::remote(),
        }
    }

    #[cfg(feature = "aws")]
    fn remote() -> Result<Self, Error> {
        let transport = crate::lambda_transport::LambdaTransport::from_env()?;
        Something::connect(Box::new(transport), ClientRequirements::default())
    }

    #[cfg(not(feature = "aws"))]
    fn remote() -> Result<Self, Error> {
        Err(Error::Config(
            "remote mode requires the \"aws\" feature, which this build does not have".to_string(),
        ))
    }

    /// Perform the handshake and cache its result.
    ///
    /// Negotiating at construction means a misconfigured client fails at
    /// startup rather than on its first real call.
    pub fn connect(
        transport: Box<dyn Transport>,
        requirements: ClientRequirements,
    ) -> Result<Self, Error> {
        let negotiated = Something::register(transport.as_ref(), &requirements)?;

        Ok(Something {
            transport,
            requirements,
            negotiated: Mutex::new(negotiated),
        })
    }

    pub fn negotiated_version(&self) -> u32 {
        self.state().version
    }

    pub fn server_build(&self) -> String {
        self.state().server_build
    }

    pub fn transport_description(&self) -> String {
        self.transport.describe()
    }

    fn state(&self) -> Negotiated {
        self.negotiated
            .lock()
            .expect("negotiation state mutex poisoned")
            .clone()
    }

    fn register(
        transport: &dyn Transport,
        requirements: &ClientRequirements,
    ) -> Result<Negotiated, Error> {
        let response = round_trip(
            transport,
            Request {
                body: Some(request::Body::Register(RegisterRequest {
                    client_build: requirements.client_build.clone(),
                })),
            },
        )?;

        match body(response)? {
            response::Body::Register(register) => negotiate(requirements, &register),
            response::Body::Error(err) => Err(Error::Server {
                code: err.code(),
                message: err.message,
            }),
            response::Body::Exec(_) => Err(Error::Protocol(
                "server answered register with an exec response".to_string(),
            )),
        }
    }

    fn exec_ping(&self, version: u32) -> Result<String, Error> {
        let response = round_trip(
            self.transport.as_ref(),
            Request {
                body: Some(request::Body::Exec(ExecRequest {
                    negotiated_version: version,
                    call: Some(exec_request::Call::Ping(PingRequest {})),
                })),
            },
        )?;

        match body(response)? {
            response::Body::Exec(exec) => match exec.output {
                Some(exec_response::Output::Ping(ping)) => Ok(ping.message),
                None => Err(Error::Protocol("exec response has no output".to_string())),
            },
            response::Body::Error(err) => Err(Error::Server {
                code: err.code(),
                message: err.message,
            }),
            response::Body::Register(_) => Err(Error::Protocol(
                "server answered exec with a register response".to_string(),
            )),
        }
    }
}

/// Hand-written because a boxed transport is not `Debug`. Shows the three
/// things worth seeing in a log line: where calls go, and what was agreed.
impl std::fmt::Debug for Something {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state();
        f.debug_struct("Something")
            .field("transport", &self.transport.describe())
            .field("negotiated_version", &state.version)
            .field("server_build", &state.server_build)
            .finish()
    }
}

impl DoSomething for Something {
    /// The handshake is a snapshot, and a redeploy can invalidate it between
    /// calls. So an `UnsupportedVersion` refusal is not a failure yet: we
    /// re-register once, retry once, and only then give up. Exactly one retry,
    /// because a server that rejects a freshly negotiated version is broken or
    /// flapping, and looping would turn that into a storm.
    fn ping(&self) -> Result<String, Error> {
        let version = self.state().version;

        match self.exec_ping(version) {
            Err(err) if err.is_unsupported_version() => {
                let renegotiated =
                    Something::register(self.transport.as_ref(), &self.requirements)?;

                *self
                    .negotiated
                    .lock()
                    .expect("negotiation state mutex poisoned") = renegotiated.clone();

                self.exec_ping(renegotiated.version)
            }
            other => other,
        }
    }
}

fn body(response: Response) -> Result<response::Body, Error> {
    response
        .body
        .ok_or_else(|| Error::Protocol("response has no body".to_string()))
}
