use something_proto::v1::RegisterResponse;
use something_proto::{range_display, version_range};

use crate::error::Error;

/// What this client needs. The server never sees these: it describes itself,
/// and the client alone decides whether that is acceptable.
///
/// Keeping the policy client-side means a server can add or retire versions
/// without knowing anything about who is calling it.
#[derive(Debug, Clone)]
pub struct ClientRequirements {
    pub protocol_min: u32,
    pub protocol_max: u32,
    pub required_capabilities: Vec<String>,
    /// Sent in the register call for diagnostics only.
    pub client_build: String,
}

impl ClientRequirements {
    pub fn new(protocol_min: u32, protocol_max: u32, required_capabilities: Vec<String>) -> Self {
        ClientRequirements {
            protocol_min,
            protocol_max,
            required_capabilities,
            client_build: default_build(),
        }
    }
}

impl Default for ClientRequirements {
    fn default() -> Self {
        ClientRequirements::new(1, 1, vec!["ping".to_string()])
    }
}

fn default_build() -> String {
    concat!("something-client/", env!("CARGO_PKG_VERSION")).to_string()
}

/// The outcome of a handshake. Cached by the facade, and re-derived whenever
/// the server tells us the cached version is no longer good.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Negotiated {
    pub version: u32,
    pub server_build: String,
}

/// Pick the highest protocol version acceptable to everyone.
///
/// "Everyone" is: the client's range, the server's inbound range, the server's
/// outbound range, and the range of every capability the client requires. We
/// intersect all of them and take the top, so adding a required capability can
/// only ever lower the result, never silently succeed at a version where that
/// capability does not exist.
pub fn negotiate(
    requirements: &ClientRequirements,
    register: &RegisterResponse,
) -> Result<Negotiated, Error> {
    let inbound = register
        .inbound
        .ok_or_else(|| Error::Protocol("register response has no inbound range".to_string()))?;
    let outbound = register
        .outbound
        .ok_or_else(|| Error::Protocol("register response has no outbound range".to_string()))?;

    let mut low = requirements.protocol_min.max(inbound.min).max(outbound.min);
    let mut high = requirements.protocol_max.min(inbound.max).min(outbound.max);

    for name in &requirements.required_capabilities {
        let Some(capability) = register.capabilities.iter().find(|c| &c.name == name) else {
            let available: Vec<&str> = register
                .capabilities
                .iter()
                .map(|c| c.name.as_str())
                .collect();
            return Err(Error::Incompatible(format!(
                "server {} does not provide required capability {name}; it has [{}]",
                register.build,
                available.join(", ")
            )));
        };

        low = low.max(capability.min_version);
        high = high.min(capability.max_version);
    }

    if low > high {
        return Err(Error::Incompatible(format!(
            "no common protocol version: client {}, server inbound {}, outbound {}{}",
            range_display(&version_range(
                requirements.protocol_min,
                requirements.protocol_max
            )),
            range_display(&inbound),
            range_display(&outbound),
            capability_ranges(requirements, register),
        )));
    }

    Ok(Negotiated {
        version: high,
        server_build: register.build.clone(),
    })
}

/// Included in the failure message only when capabilities actually constrain
/// the answer, so the common case stays readable.
fn capability_ranges(requirements: &ClientRequirements, register: &RegisterResponse) -> String {
    let parts: Vec<String> = register
        .capabilities
        .iter()
        .filter(|c| requirements.required_capabilities.contains(&c.name))
        .map(|c| format!("{} {}-{}", c.name, c.min_version, c.max_version))
        .collect();

    if parts.is_empty() {
        String::new()
    } else {
        format!(", capabilities {}", parts.join(", "))
    }
}
