"""Python-side tests. Run after `make py-develop`; they never touch AWS."""

import pytest

import something
from something import PingBackend, Something


def test_local_ping_returns_pong() -> None:
    client = Something(mode="local")

    assert client.ping() == "pong"


def test_negotiated_state_is_populated() -> None:
    client = Something(mode="local")

    assert client.negotiated_version >= 1
    assert client.server_build


def test_instance_satisfies_the_protocol() -> None:
    # The point of the pattern: callers depend on the shape, not the class.
    assert isinstance(Something(mode="local"), PingBackend)


def test_repr_names_the_transport() -> None:
    assert "loopback" in repr(Something(mode="local"))


def test_unknown_mode_is_rejected() -> None:
    # A typo must not silently fall back to running things locally.
    with pytest.raises(something.SomethingError) as excinfo:
        Something(mode="lokal")

    assert "lokal" in str(excinfo.value)


@pytest.mark.parametrize(
    "name",
    ["IncompatibleError", "TransportError", "ServerError"],
)
def test_exceptions_derive_from_the_base(name: str) -> None:
    assert issubclass(getattr(something, name), something.SomethingError)
