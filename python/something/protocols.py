"""Structural type for the facade.

PyO3 cannot project the Rust `DoSomething` trait into Python, so the protocol
is written by hand here. It exists so callers can type against the *shape* of
the backend rather than the concrete class -- which is the whole point of the
pattern: a test double, a future async client, or `Something` itself all
satisfy this, and no caller can tell them apart.
"""

from typing import Protocol, runtime_checkable


@runtime_checkable
class PingBackend(Protocol):
    """Anything that can answer a ping and describe what it negotiated."""

    def ping(self) -> str:
        """Return ``"pong"``, locally or from Lambda."""
        ...

    @property
    def negotiated_version(self) -> int:
        """Protocol version agreed at construction (or after a retry)."""
        ...

    @property
    def server_build(self) -> str:
        """Opaque build id of whatever answered the handshake."""
        ...
