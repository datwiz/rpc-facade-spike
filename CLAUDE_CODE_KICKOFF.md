# Kickoff: "Something" PoC — Rust facade over AWS Lambda with local/remote transparency

You are starting a new Rust project from scratch. Build it as a **reference implementation to share with team members**, so favour clarity, small files, and working, tested code over cleverness. Read this whole document before writing code. Nothing here has been compiled or verified yet; treat crate versions and API details as things to check against current docs (use `cargo search`, docs.rs, or `cargo add` to get current versions rather than trusting versions mentioned here).

## 1. Goal

Demonstrate a calling interface where the **caller does not know whether a function executes in-process or remotely on AWS Lambda**. The same client facade is usable from Rust and from Python (via PyO3).

The demo function is `ping()`, which returns the string `"pong"`. It is a placeholder for real "functional style" logic later, so the structure matters more than the function.

This is essentially the remote-proxy / client-stub pattern (similar in spirit to CORBA/DCOM, but restricted to stateless function calls with serializable inputs and outputs).

## 2. Settled design decisions (implement these; flag anything you think is wrong rather than silently changing it)

### 2.1 One code path, swappable transport
- A `Transport` trait: `fn call(&self, request: &[u8]) -> Result<Vec<u8>, TransportError>` (bytes in, bytes out). Keep it **synchronous** for the PoC so the PyO3 layer stays simple. The Lambda transport owns a small tokio runtime and `block_on`s the AWS SDK call. Document this trade-off in the README (it will misbehave if called from inside an async context).
- Two implementations:
  - `LoopbackTransport`: runs the handler **in-process, but through the full protobuf encode/decode path**. This is the "local" mode. It deliberately keeps the local path honest (serialization, versioning and error handling behave identically to remote), so the two modes cannot drift apart.
  - `LambdaTransport` (behind a cargo feature, default on): invokes a Lambda via `aws-sdk-lambda`.
- The facade struct `Something` implements a trait `DoSomething { fn ping(&self) -> Result<String, Error>; }`. The mode is chosen once at construction, from the `SOMETHING_MODE` env var (`local` default, or `remote`), with an explicit constructor override for tests and Python.
- Note for the user: the original sketch had `do_something_local()` and `do_something_remote()` methods with env-var dispatch inside the struct. This design achieves the same outcome with one code path and the dispatch expressed as a transport choice. If the user prefers the explicit methods, change it; otherwise keep this.

### 2.2 Wire protocol: protobuf with `oneof` (no implicit operations)
Operations are explicit via `oneof`. Do **not** overload an optional field (for example "version absent means handshake"): it silently misroutes on bugs, and it cannot express a third operation.

Sketch (refine as needed) in `proto/something/v1/something.proto`:

```protobuf
syntax = "proto3";
package something.v1;

message Request {
  oneof body {
    RegisterRequest register = 1;
    ExecRequest exec = 2;
  }
}
message RegisterRequest { string client_build = 1; }   // diagnostics only

message ExecRequest {
  uint32 negotiated_version = 1;   // required; server rejects 0
  oneof call { PingRequest ping = 2; }
}
message PingRequest {}

message Response {
  oneof body {
    RegisterResponse register = 1;
    ExecResponse exec = 2;
    ErrorResponse error = 3;
  }
}
message RegisterResponse {
  string build = 1;
  repeated Capability capabilities = 2;
  VersionRange inbound = 3;        // request versions the server accepts
  VersionRange outbound = 4;       // response versions the server can emit
  map<string, string> limits = 5;  // e.g. max payload; informational
}
message Capability { string name = 1; uint32 min_version = 2; uint32 max_version = 3; }
message VersionRange { uint32 min = 1; uint32 max = 2; }
message ExecResponse { oneof output { PingResponse ping = 1; } }  // avoid a oneof named `result`
message PingResponse { string message = 1; }

enum ErrorCode {
  ERROR_CODE_UNSPECIFIED = 0;
  ERROR_CODE_INVALID_REQUEST = 1;
  ERROR_CODE_MISSING_VERSION = 2;
  ERROR_CODE_UNSUPPORTED_VERSION = 3;
  ERROR_CODE_UNSUPPORTED_CAPABILITY = 4;
  ERROR_CODE_INTERNAL = 5;
}
message ErrorResponse { ErrorCode code = 1; string message = 2; }
```

Generate Rust with `prost`/`prost-build`. Prefer `protoc-bin-vendored` in `build.rs` so teammates do not need a system `protoc`. Keep `.proto` files in a top-level `proto/` directory so non-Rust consumers can use them.

### 2.3 Lambda envelope
Lambda payloads are JSON. Wrap protobuf bytes as `{"payload": "<base64 of protobuf bytes>"}` in both directions. The envelope is the one part that cannot be negotiated, so keep it tiny and stable. Put the `Envelope` type in the shared proto crate so the Lambda binary and the client both use it.

### 2.4 Negotiation: the server advertises, the client decides
- `RegisterRequest` returns a `RegisterResponse` describing what the server supports (capabilities with version ranges, inbound and outbound protobuf version ranges, build id, limits). The client applies its own policy. The server never answers yes or no to a client's requirements.
- Client requirements: `{ protocol_min, protocol_max, required_capabilities: Vec<String> }`. Capability `min_version` and `max_version` mean "the range of negotiated protocol versions for which this capability is available".
- Negotiated version = the highest version inside the intersection of the client range, the server inbound range, the server outbound range, and the range of every required capability. If the intersection is empty, or a required capability is missing, return `Error::Incompatible` with a message that shows the actual ranges (for example "server inbound 1-2, need 4").
- The server default in the PoC is protocol version range 1..=1 with capability `ping`. Tests should construct handlers with other ranges to exercise negotiation. Do not invent fake behaviour differences per version.

### 2.5 Startup negotiation is a snapshot, so exec is the real guarantee
- Every `ExecRequest` carries `negotiated_version`. The server rejects `0` with `MISSING_VERSION` and anything outside its ranges (or the capability's range) with `UNSUPPORTED_VERSION`.
- On `UNSUPPORTED_VERSION` the client **re-registers once, re-negotiates, and retries once**; then it surfaces the error. This handles redeploys and a different execution environment answering.
- The client caches the negotiation result (version, server build) in the `Something` struct at construction (`connect` performs the register call).

### 2.6 Error model
- Application-level failures (`InvalidRequest`, `UnsupportedVersion`, and so on) are returned **in the response body** as `Response.error` with a normal successful Lambda invocation.
- Lambda function errors (crashes, timeouts, throttling) are transport errors. The client distinguishes them: `Error::Transport`, `Error::Incompatible`, `Error::Server { code, message }`, `Error::Protocol` (unexpected or empty response body), `Error::Decode`.
- The handler must never panic on bad input. Undecodable bytes or an empty `body` return an `InvalidRequest` error response.

### 2.7 Lambda binary lifecycle
- `main()` runs once per execution environment (cold start). Do expensive setup there and pass it to the handler via closure. Keep the handler thin: decode envelope, call `Handler::handle_bytes`, encode envelope.
- The business logic and protocol handling live in a **pure library crate with no Lambda or AWS dependencies**, so the same code backs the Lambda binary and the loopback transport.
- Invoke with a **qualifier** (published version or alias), configurable via `SOMETHING_LAMBDA_QUALIFIER`. Never rely on `$LATEST`, because the register call and later exec calls must hit the same code. Function name comes from `SOMETHING_LAMBDA_FUNCTION`. Set an explicit timeout on the client.
- Build and deploy with `cargo-lambda` (arm64 / `provided.al2023`).

### 2.8 Python exposure (PyO3 + maturin)
- PyO3 cannot turn a Rust trait into a Python `Protocol`. Expose a concrete `#[pyclass(name = "Something")]` wrapping `Arc<Something>` (or the facade type), and **hand-write the `Protocol`** in the Python half of a mixed maturin project.
- Layout: `python-source = "python"`, native module `something._something`, Python package `something` re-exporting the class and exceptions, with a `PingBackend` `typing.Protocol`, a `py.typed` marker, and a `.pyi` stub for the native module (`pyo3-stub-gen` is optional; a handwritten stub is fine).
- Python API: `Something(mode: str | None = None)`, `.ping() -> str`, read-only `negotiated_version` and `server_build`. The `mode` argument overrides `SOMETHING_MODE`.
- Release the GIL around blocking calls. **Check the installed PyO3 version:** `Python::allow_threads` was renamed `Python::detach` in newer releases (0.26+), so use whichever the pinned version provides.
- Define a Python exception hierarchy with `create_exception!`: `SomethingError` (base), `IncompatibleError`, `TransportError`, `ServerError`. Convert from the Rust `Error` enum.
- Keep the PyO3 wrapper in its **own crate** so the core crates have no PyO3 dependency.

## 3. Suggested layout

```
something-poc/
  Cargo.toml                 # workspace, resolver = "2"
  README.md
  proto/something/v1/something.proto
  crates/
    proto/     # prost-generated types + Envelope (serde_json + base64)
    core/      # pure logic + Handler (register/exec), no Lambda/AWS deps
    client/    # Transport trait, LoopbackTransport, LambdaTransport (feature "aws"),
               # negotiation, Something facade, DoSomething trait, Error
    lambda/    # bin: lambda_runtime + tokio; thin adapter over core::Handler
    py/        # cdylib: PyO3 wrapper (module name _something)
  python/something/
    __init__.py  _something.pyi  py.typed  protocols.py
  examples/    # rust example + python example script
```

Make sure `cargo test` at the workspace root works **without AWS credentials or network access** (default members and feature flags should allow this). AWS-dependent tests, if any, must be `#[ignore]`d and documented.

## 4. Behaviour the tests must cover

Core handler:
- `ping` exec at a valid version returns `"pong"`.
- Exec with `negotiated_version = 0` returns `MISSING_VERSION`.
- Exec with a version outside the server range returns `UNSUPPORTED_VERSION`.
- Register returns the configured capabilities and ranges.
- Garbage bytes and an empty `body` return `INVALID_REQUEST` (no panic).

Client:
- Loopback `ping()` returns `"pong"` end to end through protobuf.
- Negotiation picks the highest common version (for example server 1..=3, client 1..=2 gives 2).
- Disjoint ranges or a missing required capability give `Incompatible` with a useful message.
- **Renegotiation:** use a test transport whose server can be swapped mid-test. Connect against a server supporting 1..=2 (negotiates 2), swap to a server supporting 1..=1, call `ping()`, and assert it re-registers, drops to 1, and succeeds. Also assert it gives up after one retry if the server keeps rejecting.
- `SOMETHING_MODE` parsing (unknown values are an error, not a silent fallback).

Envelope: base64 round trip, plus malformed input rejected.

Python: a `pytest` test (run after `maturin develop`) covering `Something(mode="local").ping() == "pong"`, that `negotiated_version` is set, and that the exception classes exist and derive from `SomethingError`.

## 5. Tooling and workflow

- Use current stable Rust, edition 2021 or newer, and pin dependency versions through a committed `Cargo.lock`.
- Provide a `justfile` or `Makefile` with: `test`, `lambda-build` (`cargo lambda build --release --arm64`), `lambda-deploy`, `py-develop` (`maturin develop`), `py-test`.
- Run `cargo fmt`, `cargo clippy -- -D warnings`, and `cargo test` before declaring anything done. Actually run them and report the results; do not claim success without running.
- The AWS pieces (`LambdaTransport`, the Lambda binary) may not be testable without an account. Compile them, and if you cannot exercise them, say so explicitly in the README and in your summary. Suggest a manual smoke test using `cargo lambda watch` and `cargo lambda invoke` for the handler, and a documented procedure for the real deploy.

## 6. Constraints on scope

- Build only what is described. No extra capabilities, no async client API, no retries beyond the single renegotiation retry, no config framework.
- README must cover: what the pattern is, architecture diagram (ASCII is fine), how to run locally, how to deploy the Lambda, how to use from Rust and Python, the leaky-abstraction caveats (latency, timeouts and throttling, cold starts, payload limits of 6 MB sync, serialization, version skew), and how to add a new capability.
- Keep comments focused on *why*, since this is meant to be read by teammates.

## 7. First steps

1. Confirm the toolchain (`rustc --version`, `cargo --version`, `cargo lambda --version`, `python3 --version`, `maturin --version`) and report what is missing.
2. Scaffold the workspace, proto, and `proto` crate; get `cargo build` green.
3. Implement `core` and its tests, then `client` with loopback and its tests, then `lambda`, then `py`.
4. Stop after each stage if something in this document conflicts with what you find in the real crate APIs, and tell me what you changed and why.
5. Finish with a short summary: what was verified by running it, what was only compiled, and open questions.

## 8. Open questions to raise with the user if they matter (otherwise use the defaults given)

- Sync facade (default) or async? Default: sync.
- Should the Python package also ship a pure-Python fallback? Default: no.
- Payload-size handling for large inputs (S3 reference) is out of scope for now, but note where it would plug in.
