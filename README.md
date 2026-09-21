# something-poc

A calling interface where **the caller cannot tell whether the function ran in
this process or in an AWS Lambda execution environment**.

```rust
let client = Something::from_env()?;   // SOMETHING_MODE decides, once
client.ping()?;                        // "pong", from here or from us-east-1
```

The demo operation is `ping() -> "pong"`. It is a placeholder: the structure is
the deliverable, not the function.

## The pattern

This is the classic remote-proxy / client-stub arrangement (CORBA, DCOM, gRPC
stubs), narrowed to what actually survives contact with Lambda: **stateless
calls with serializable inputs and outputs**. No object references, no
callbacks, no sessions.

Two ideas carry it:

1. **One code path.** `Something` always encodes a protobuf request, hands the
   bytes to a `Transport`, and decodes protobuf bytes back. Local mode is not a
   shortcut around that path — it is a transport that calls the handler
   in-process, *through the same encode/decode*. Deliberately wasteful, so the
   two modes cannot quietly diverge in serialization, versioning or error
   handling.
2. **The server describes, the client decides.** Registration returns what the
   server supports; the client applies its own policy against it. The server
   never answers yes or no to a client's requirements, so it needs to know
   nothing about its callers.

## Architecture

```
  caller (Rust)            caller (Python)
        |                        |
        |                  something.Something  ──┐  hand-written
        |                  something.PingBackend ─┘  typing.Protocol
        |                        |
        |                  crates/py  (PyO3, cdylib _something)
        |                        |
        +-----------+------------+
                    |
              Something  (crates/client)
              - handshake, cached negotiated version
              - one retry on UNSUPPORTED_VERSION
                    |
              Transport  (bytes in, bytes out)
                    |
        +-----------+------------------------+
        |                                    |
  LoopbackTransport                    LambdaTransport
  (in-process, full codec)             (aws-sdk-lambda, owns a tokio runtime)
        |                                    |
        |                            JSON {"payload": base64(protobuf)}
        |                                    |
        |                            AWS Lambda (provided.al2023, arm64)
        |                            crates/lambda  bootstrap
        |                                    |
        +-----------+------------------------+
                    |
              Handler  (crates/core)  ← pure library, no AWS, no I/O
              register / exec / error responses
```

`crates/core` has no Lambda, AWS, async or I/O dependencies. That is what lets
the same code answer both an in-process call and a real invocation.

### Layout

| Path | What it is |
| --- | --- |
| `proto/something/v1/something.proto` | The wire contract, kept at the top level so non-Rust consumers can use it |
| `crates/proto` | prost-generated types plus the Lambda JSON `Envelope` |
| `crates/core` | `Handler`: protocol handling and business logic, pure |
| `crates/client` | `Transport`, both transports, negotiation, the `Something` facade |
| `crates/lambda` | `bootstrap` binary: a thin adapter over `core` |
| `crates/py` | PyO3 bindings (its own crate, so nothing else depends on Python) |
| `python/something` | Hand-written Python: re-exports, `PingBackend`, stubs |

## Running it

```bash
make test        # Rust suite; no AWS, no credentials, no network at run time
make lint        # fmt --check + clippy -D warnings, including a no-AWS build
cargo run -p something-client --example ping
```

Python:

```bash
make venv
make py-develop  # maturin develop into .venv
make py-test
python examples/ping.py
```

```python
from something import Something

client = Something(mode="local")   # or omit mode and set SOMETHING_MODE
client.ping()                      # 'pong'
client.negotiated_version          # 1
```

### Configuration

| Variable | Meaning |
| --- | --- |
| `SOMETHING_MODE` | `local` (default) or `remote`. An unknown value is an error, never a silent fall back to local. |
| `SOMETHING_LAMBDA_FUNCTION` | Function name. Required in remote mode. |
| `SOMETHING_LAMBDA_QUALIFIER` | Published version or alias. **Required** — see below. |
| `SOMETHING_LAMBDA_TIMEOUT_SECS` | Client-side invoke timeout, default 30. |

The qualifier has no default on purpose. `$LATEST` can change between the
register call and the exec calls that follow it, which is exactly the version
skew this protocol exists to make visible.

## Deploying the Lambda

Needs [`cargo-lambda`](https://www.cargo-lambda.info) (`brew install
cargo-lambda`, or `cargo install cargo-lambda`; cross-compiling from macOS also
wants `zig`).

```bash
make lambda-build                                   # arm64, provided.al2023
make lambda-deploy FUNCTION=something-poc ALIAS=live IAM_ROLE=arn:aws:iam::...
```

`lambda-deploy` publishes a numbered version and moves the alias to it, because
the client invokes through a qualifier.

Then:

```bash
SOMETHING_MODE=remote \
SOMETHING_LAMBDA_FUNCTION=something-poc \
SOMETHING_LAMBDA_QUALIFIER=live \
  cargo run -p something-client --example ping
```

### Smoke-testing the handler without deploying

```bash
make lambda-watch     # serves the handler on :9000
# in another shell — base64 of an exec/ping request at version 1:
cargo lambda invoke bootstrap --data-ascii '{"payload":"EgQIARIA"}'
```

A successful response is `{"payload":"..."}` whose decoded protobuf contains
`pong`.

## The wire protocol

Operations are explicit `oneof` variants. No field is overloaded to mean "this
is a different operation" — an absent body is an error, not a default. That
costs a few bytes and removes a class of silent misrouting.

An `ExecRequest` always carries `negotiated_version`, and the server re-checks
it on every call. Registration is only a snapshot: a redeploy, or a different
execution environment answering, can invalidate it between the handshake and
the call.

**Negotiated version** = the highest version in the intersection of the client
range, the server's inbound range, the server's outbound range, and the range
of every required capability. An empty intersection, or a missing capability,
is `Error::Incompatible` with the actual ranges in the message (`client 4-5,
server inbound 1-2, ...`), because "incompatible" alone leaves the reader
guessing which side has to move.

A capability's `min_version`/`max_version` are the *protocol* versions it is
available for — not a version of the capability itself. So a capability can be
retired without retiring the protocol version it lived in.

### Errors

| Kind | Where it shows up |
| --- | --- |
| Application-level refusal (`InvalidRequest`, `UnsupportedVersion`, …) | In the response body, on an otherwise **successful** invocation → `Error::Server` |
| Crash, timeout, throttling, bad credentials | Lambda function error or SDK failure → `Error::Transport` |
| No workable version or capability | `Error::Incompatible` |
| Well-formed but nonsensical response | `Error::Protocol` |
| Undecodable bytes | `Error::Decode` |
| Bad env configuration | `Error::Config` |

The handler never panics on bad input: garbage bytes and an empty body both
come back as `INVALID_REQUEST`.

On `UNSUPPORTED_VERSION` the client re-registers **once**, renegotiates, and
retries **once**, then surfaces the error. A server that rejects a version it
just advertised is broken or flapping, and looping would turn that into a
storm.

## Where the abstraction leaks

Location transparency is a useful lie. The places it fails:

- **Latency.** In-process is nanoseconds; a warm Lambda invoke is single-digit
  to tens of milliseconds, a cold start hundreds of milliseconds to seconds.
  Loops that are free locally are not free remotely.
- **Cold starts.** The first call after a deploy or a scale-out pays init. It
  is also the call most likely to see a *different* build than the one you
  registered against — hence the retry.
- **Timeouts and throttling.** Remote calls can fail for reasons local ones
  cannot: concurrency limits, account throttling, credential expiry. These
  arrive as `Error::Transport`, which has no local equivalent.
- **Payload limits.** Synchronous invoke caps request and response at 6 MB, and
  base64 adds ~33% over the raw protobuf. The server advertises
  `max_payload_bytes` in `limits`. Large payloads would need an S3 reference in
  the envelope — out of scope, but the `Envelope` type is where it plugs in.
- **Serialization.** Arguments must be serializable and are *copied*. No shared
  mutable state, no references, no callbacks.
- **Version skew.** Two processes, deployed separately, drift. The whole
  register/negotiate/re-check mechanism exists for this.
- **The sync facade blocks.** `LambdaTransport` owns a current-thread tokio
  runtime and `block_on`s the SDK. **Calling it from inside an async runtime
  panics.** This keeps the PyO3 layer simple; an async facade is the fix if
  this outgrows a PoC.

## Adding a capability

Say `echo(text) -> text`:

1. **`proto/something/v1/something.proto`** — add `EchoRequest`/`EchoResponse`,
   and a variant to each `oneof`: `EchoRequest echo = 3;` in `ExecRequest.call`
   and `EchoResponse echo = 2;` in `ExecResponse.output`. Never renumber or
   reuse a field number.
2. **`crates/core/src/handler.rs`** — match the new `exec_request::Call`
   variant, and add `capability("echo", min, max)` to `HandlerConfig`. The
   `check_capability` call is what makes the version gate apply.
3. **`crates/client/src/something.rs`** — add the method, and put `echo` in
   `ClientRequirements::required_capabilities` if callers cannot work without
   it. Required capabilities can only lower the negotiated version, never raise
   it.
4. **Python** — add the method to `PySomething`, to `_something.pyi`, and to
   `PingBackend` (or a new protocol).
5. **Tests** — the exec path, the version gate, and a negotiation case.

Bump the protocol version only when the *meaning* of existing messages changes.
Adding a capability does not require it; capability ranges already express
"this exists here and not there".

## Environment notes

- **Rust 1.94+** is required: the current AWS SDK crates set that as their MSRV.
- **`protoc` is not needed.** `crates/proto/build.rs` uses
  `protoc-bin-vendored`, so everyone generates with the same compiler.
- **`cargo test` at the workspace root skips `crates/py`.** With PyO3's
  `extension-module` feature on, that crate intentionally leaves Python symbols
  undefined and cannot be linked into a test binary. maturin builds it by
  manifest path instead. `make lint` still clippy-checks it.
- **macOS: "You have not agreed to the Xcode license agreements."** If
  `xcode-select -p` points at `Xcode.app` and you have not accepted its
  licence, linking fails. Either accept it (`sudo xcodebuild -license`), point
  at the command line tools (`sudo xcode-select -s
  /Library/Developer/CommandLineTools`), or per-shell:
  `export DEVELOPER_DIR=/Library/Developer/CommandLineTools`.

## What has and has not been exercised

Verified by running: the Rust suite (24 tests — handler, envelope, negotiation,
renegotiation, mode parsing), `cargo clippy -D warnings` (default, no-default
and PyO3 builds), `cargo fmt --check`, and the Python suite (8 tests) after
`maturin develop`.

Compiled but **never executed against AWS**: `LambdaTransport` and the
`bootstrap` binary. No account was available, so `cargo lambda build`, the
deploy path and a real remote `ping()` are unverified. Use the smoke test above
before trusting them.
