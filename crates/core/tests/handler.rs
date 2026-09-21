//! Server-side behaviour. These tests talk to the handler in its real
//! currency — encoded bytes — so they cover decode failures too.

use prost::Message;
use proto::v1::{
    exec_request, exec_response, request, response, ErrorCode, ExecRequest, PingRequest,
    RegisterRequest, Request, Response,
};
use something_core::{proto, Handler, HandlerConfig};

fn handle(handler: &Handler, request: Request) -> Response {
    Response::decode(handler.handle_bytes(&request.encode_to_vec()).as_slice())
        .expect("handler always emits a decodable Response")
}

fn exec_ping(version: u32) -> Request {
    Request {
        body: Some(request::Body::Exec(ExecRequest {
            negotiated_version: version,
            call: Some(exec_request::Call::Ping(PingRequest {})),
        })),
    }
}

fn expect_error(response: Response) -> (ErrorCode, String) {
    match response.body {
        Some(response::Body::Error(err)) => (err.code(), err.message),
        other => panic!("expected an error response, got {other:?}"),
    }
}

#[test]
fn ping_at_a_valid_version_returns_pong() {
    let handler = Handler::default();

    let response = handle(&handler, exec_ping(1));

    match response.body {
        Some(response::Body::Exec(exec)) => match exec.output {
            Some(exec_response::Output::Ping(ping)) => assert_eq!(ping.message, "pong"),
            other => panic!("expected a ping output, got {other:?}"),
        },
        other => panic!("expected an exec response, got {other:?}"),
    }
}

#[test]
fn exec_without_a_version_is_rejected() {
    let handler = Handler::default();

    let (code, message) = expect_error(handle(&handler, exec_ping(0)));

    assert_eq!(code, ErrorCode::MissingVersion);
    assert!(message.contains("negotiated_version"), "message: {message}");
}

#[test]
fn exec_outside_the_server_range_is_rejected() {
    let handler = Handler::new(HandlerConfig::new("test").with_protocol_range(1, 2));

    let (code, message) = expect_error(handle(&handler, exec_ping(3)));

    assert_eq!(code, ErrorCode::UnsupportedVersion);
    // The actual ranges belong in the message: "unsupported" alone leaves the
    // caller guessing which side moved.
    assert!(message.contains("1-2"), "message: {message}");
}

#[test]
fn exec_outside_the_capability_range_is_rejected() {
    let handler = Handler::new(
        HandlerConfig::new("test")
            .with_protocol_range(1, 3)
            .with_capabilities(vec![something_core::capability("ping", 1, 2)]),
    );

    let (code, message) = expect_error(handle(&handler, exec_ping(3)));

    assert_eq!(code, ErrorCode::UnsupportedVersion);
    assert!(message.contains("capability ping"), "message: {message}");
}

#[test]
fn exec_for_a_missing_capability_is_rejected() {
    let handler = Handler::new(HandlerConfig::new("test").with_capabilities(vec![]));

    let (code, _) = expect_error(handle(&handler, exec_ping(1)));

    assert_eq!(code, ErrorCode::UnsupportedCapability);
}

#[test]
fn register_reports_the_configured_ranges_and_capabilities() {
    let handler = Handler::new(HandlerConfig::new("build-xyz").with_protocol_range(2, 4));

    let response = handle(
        &handler,
        Request {
            body: Some(request::Body::Register(RegisterRequest {
                client_build: "test-client".to_string(),
            })),
        },
    );

    match response.body {
        Some(response::Body::Register(register)) => {
            assert_eq!(register.build, "build-xyz");
            assert_eq!(register.inbound.unwrap().max, 4);
            assert_eq!(register.outbound.unwrap().min, 2);
            assert_eq!(register.capabilities.len(), 1);
            assert_eq!(register.capabilities[0].name, "ping");
            assert!(register.limits.contains_key("max_payload_bytes"));
        }
        other => panic!("expected a register response, got {other:?}"),
    }
}

#[test]
fn garbage_bytes_do_not_panic() {
    let handler = Handler::default();

    // Field 1 declared as length-delimited with a length that runs off the end.
    let response = Response::decode(handler.handle_bytes(&[0x0a, 0xff, 0xff]).as_slice()).unwrap();

    assert_eq!(expect_error(response).0, ErrorCode::InvalidRequest);
}

#[test]
fn an_empty_body_is_an_error_not_a_default_operation() {
    let handler = Handler::default();

    // An empty buffer decodes cleanly as `Request { body: None }` — this is
    // exactly the case a `oneof` makes visible instead of silently routing.
    let response = Response::decode(handler.handle_bytes(&[]).as_slice()).unwrap();

    assert_eq!(expect_error(response).0, ErrorCode::InvalidRequest);
}

#[test]
fn exec_without_a_call_is_an_error() {
    let handler = Handler::default();

    let response = handle(
        &handler,
        Request {
            body: Some(request::Body::Exec(ExecRequest {
                negotiated_version: 1,
                call: None,
            })),
        },
    );

    assert_eq!(expect_error(response).0, ErrorCode::InvalidRequest);
}

#[test]
fn the_payload_documented_in_the_readme_is_an_exec_ping() {
    // Pins the base64 in the README's `cargo lambda invoke` smoke test. If the
    // proto changes shape, this fails before the docs go stale.
    let encoded = exec_ping(1).encode_to_vec();
    assert_eq!(encoded, vec![0x12, 0x04, 0x08, 0x01, 0x12, 0x00]);

    let response = Response::decode(Handler::default().handle_bytes(&encoded).as_slice()).unwrap();
    match response.body {
        Some(response::Body::Exec(exec)) => match exec.output {
            Some(exec_response::Output::Ping(ping)) => assert_eq!(ping.message, "pong"),
            other => panic!("expected a ping output, got {other:?}"),
        },
        other => panic!("expected an exec response, got {other:?}"),
    }
}
