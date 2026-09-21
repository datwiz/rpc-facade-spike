"""Same code, either mode.

    python examples/ping.py                     # local, in-process
    SOMETHING_MODE=remote \
    SOMETHING_LAMBDA_FUNCTION=something-poc \
    SOMETHING_LAMBDA_QUALIFIER=live \
    python examples/ping.py                     # remote, on AWS

Note that nothing below mentions either mode.
"""

from something import PingBackend, Something


def greet(backend: PingBackend) -> str:
    """Typed against the protocol, so it cannot depend on where it runs."""
    return backend.ping()


if __name__ == "__main__":
    client = Something()
    print(repr(client))
    print(greet(client))
