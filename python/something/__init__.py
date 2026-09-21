"""Python face of the Rust facade.

The import surface is deliberately identical in both modes: nothing here, and
nothing a caller writes, changes depending on whether the work runs in this
process or in a Lambda execution environment.

    >>> from something import Something
    >>> Something(mode="local").ping()
    'pong'
"""

from ._something import (
    IncompatibleError,
    ServerError,
    Something,
    SomethingError,
    TransportError,
)
from .protocols import PingBackend

__all__ = [
    "IncompatibleError",
    "PingBackend",
    "ServerError",
    "Something",
    "SomethingError",
    "TransportError",
]
