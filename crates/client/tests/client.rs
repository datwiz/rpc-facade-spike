//! Client behaviour, all of it offline: the "remote" cases run against the
//! test transport, not AWS.

mod support;

use something_client::proto::v1::ErrorCode;
use something_client::{
    capability, ClientRequirements, DoSomething, Error, Handler, HandlerConfig, LoopbackTransport,
    Mode, Something,
};
use support::TestServer;

fn requirements(min: u32, max: u32) -> ClientRequirements {
    ClientRequirements::new(min, max, vec!["ping".to_string()])
}

/// A server that supports `min..=max` for both the protocol and `ping`.
/// Capability ranges are set explicitly because `with_protocol_range` leaves
/// them alone by design.
fn server(min: u32, max: u32) -> Handler {
    Handler::new(
        HandlerConfig::new("test-server")
            .with_protocol_range(min, max)
            .with_capabilities(vec![capability("ping", min, max)]),
    )
}

#[test]
fn loopback_ping_round_trips_through_protobuf() {
    let something = Something::with_mode(Mode::Local).unwrap();

    assert_eq!(something.ping().unwrap(), "pong");
    assert_eq!(something.negotiated_version(), 1);
    assert!(something.server_build().contains("something-core"));
}

#[test]
fn negotiation_picks_the_highest_common_version() {
    let something = Something::connect(
        Box::new(LoopbackTransport::new(server(1, 3))),
        requirements(1, 2),
    )
    .unwrap();

    assert_eq!(something.negotiated_version(), 2);
    assert_eq!(something.ping().unwrap(), "pong");
}

#[test]
fn disjoint_ranges_are_incompatible() {
    let error = Something::connect(
        Box::new(LoopbackTransport::new(server(1, 2))),
        requirements(4, 5),
    )
    .unwrap_err();

    match error {
        Error::Incompatible(message) => {
            // Both sides' actual ranges, so the reader can tell who to move.
            assert!(message.contains("client 4-5"), "message: {message}");
            assert!(message.contains("inbound 1-2"), "message: {message}");
        }
        other => panic!("expected Incompatible, got {other:?}"),
    }
}

#[test]
fn a_missing_required_capability_is_incompatible() {
    let handler = Handler::new(HandlerConfig::new("test-server").with_capabilities(vec![]));
    let error = Something::connect(
        Box::new(LoopbackTransport::new(handler)),
        ClientRequirements::new(1, 1, vec!["ping".to_string()]),
    )
    .unwrap_err();

    match error {
        Error::Incompatible(message) => {
            assert!(message.contains("ping"), "message: {message}");
        }
        other => panic!("expected Incompatible, got {other:?}"),
    }
}

#[test]
fn a_capability_range_can_lower_the_negotiated_version() {
    let handler = Handler::new(
        HandlerConfig::new("test-server")
            .with_protocol_range(1, 3)
            .with_capabilities(vec![capability("ping", 1, 2)]),
    );

    let something = Something::connect(
        Box::new(LoopbackTransport::new(handler)),
        requirements(1, 3),
    )
    .unwrap();

    assert_eq!(something.negotiated_version(), 2);
}

#[test]
fn a_redeployed_server_triggers_one_renegotiation_and_succeeds() {
    let server_handle = TestServer::new(server(1, 2));
    let something =
        Something::connect(Box::new(server_handle.transport()), requirements(1, 2)).unwrap();
    assert_eq!(something.negotiated_version(), 2);

    // The deploy that invalidates the handshake.
    server_handle.swap(server(1, 1));

    assert_eq!(something.ping().unwrap(), "pong");
    assert_eq!(
        something.negotiated_version(),
        1,
        "should have dropped to 1"
    );
    assert_eq!(
        server_handle.register_calls(),
        2,
        "connect, then re-register"
    );
    assert_eq!(server_handle.exec_calls(), 2, "rejected call, then retry");
}

#[test]
fn a_server_that_keeps_rejecting_gives_up_after_one_retry() {
    let server_handle = TestServer::new(server(1, 1));
    let something =
        Something::connect(Box::new(server_handle.transport()), requirements(1, 1)).unwrap();

    server_handle.reject_every_exec();

    match something.ping().unwrap_err() {
        Error::Server { code, .. } => assert_eq!(code, ErrorCode::UnsupportedVersion),
        other => panic!("expected a server error, got {other:?}"),
    }

    // No storm: exactly one extra handshake and one extra call.
    assert_eq!(server_handle.register_calls(), 2);
    assert_eq!(server_handle.exec_calls(), 2);
}

#[test]
fn unknown_modes_are_rejected_rather_than_defaulted() {
    assert_eq!("local".parse::<Mode>().unwrap(), Mode::Local);
    assert_eq!("  REMOTE ".parse::<Mode>().unwrap(), Mode::Remote);

    match "lokal".parse::<Mode>() {
        Err(Error::Config(message)) => assert!(message.contains("lokal"), "message: {message}"),
        other => panic!("expected a config error, got {other:?}"),
    }
    assert!("".parse::<Mode>().is_err());
}

#[test]
fn an_unset_mode_defaults_to_local() {
    // The only test that touches this variable, so the process-wide mutation
    // cannot race another one.
    std::env::remove_var("SOMETHING_MODE");
    assert_eq!(Mode::from_env().unwrap(), Mode::Local);

    std::env::set_var("SOMETHING_MODE", "nonsense");
    assert!(Mode::from_env().is_err());
    std::env::remove_var("SOMETHING_MODE");
}
