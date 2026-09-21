//! A transport whose server can be replaced mid-test, so version skew from a
//! redeploy can be reproduced without AWS.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use prost::Message;
use something_client::proto::v1::{request, response, ErrorCode, ErrorResponse, Request, Response};
use something_client::{Handler, Transport, TransportError};

#[derive(Default)]
pub struct TestServer {
    handler: Mutex<Handler>,
    /// Simulates a server that rejects every version it is offered, including
    /// one it just advertised.
    reject_exec: AtomicBool,
    register_calls: AtomicUsize,
    exec_calls: AtomicUsize,
}

impl TestServer {
    pub fn new(handler: Handler) -> Arc<Self> {
        Arc::new(TestServer {
            handler: Mutex::new(handler),
            ..TestServer::default()
        })
    }

    /// Stand in for a redeploy between calls.
    pub fn swap(&self, handler: Handler) {
        *self.handler.lock().unwrap() = handler;
    }

    pub fn reject_every_exec(&self) {
        self.reject_exec.store(true, Ordering::SeqCst);
    }

    pub fn register_calls(&self) -> usize {
        self.register_calls.load(Ordering::SeqCst)
    }

    pub fn exec_calls(&self) -> usize {
        self.exec_calls.load(Ordering::SeqCst)
    }

    pub fn transport(self: &Arc<Self>) -> TestTransport {
        TestTransport {
            server: Arc::clone(self),
        }
    }
}

pub struct TestTransport {
    server: Arc<TestServer>,
}

impl Transport for TestTransport {
    fn call(&self, request: &[u8]) -> Result<Vec<u8>, TransportError> {
        let decoded = Request::decode(request).expect("tests only send valid requests");

        match decoded.body {
            Some(request::Body::Register(_)) => {
                self.server.register_calls.fetch_add(1, Ordering::SeqCst);
            }
            Some(request::Body::Exec(_)) => {
                self.server.exec_calls.fetch_add(1, Ordering::SeqCst);

                if self.server.reject_exec.load(Ordering::SeqCst) {
                    return Ok(Response {
                        body: Some(response::Body::Error(ErrorResponse {
                            code: ErrorCode::UnsupportedVersion as i32,
                            message: "this server rejects everything".to_string(),
                        })),
                    }
                    .encode_to_vec());
                }
            }
            None => {}
        }

        Ok(self.server.handler.lock().unwrap().handle_bytes(request))
    }

    fn describe(&self) -> String {
        "test".to_string()
    }
}
