//! Python bindings for the facade.
//!
//! PyO3 cannot project a Rust trait onto a Python `typing.Protocol`, so this
//! module exposes the concrete class and the Python package hand-writes the
//! protocol (see `python/something/protocols.py`). That split is why the
//! binding crate stays this thin.

use pyo3::create_exception;
use pyo3::exceptions::PyException;
use pyo3::prelude::*;
use std::sync::Arc;

use something_client::{DoSomething, Error, Mode, Something};

create_exception!(
    _something,
    SomethingError,
    PyException,
    "Base class for every error raised by this package."
);
create_exception!(
    _something,
    IncompatibleError,
    SomethingError,
    "No protocol version or capability set satisfies both sides."
);
create_exception!(
    _something,
    TransportError,
    SomethingError,
    "The call did not complete: crash, timeout, throttling, bad credentials."
);
create_exception!(
    _something,
    ServerError,
    SomethingError,
    "The server understood the request and refused it."
);

/// Maps the Rust error enum onto the Python hierarchy.
///
/// `Protocol`, `Decode` and `Config` share the base class: they are all
/// "something is wrong with this setup", and giving each its own class would
/// invite callers to branch on distinctions they cannot act on.
fn to_py_error(error: Error) -> PyErr {
    let message = error.to_string();
    match error {
        Error::Transport(_) => TransportError::new_err(message),
        Error::Incompatible(_) => IncompatibleError::new_err(message),
        Error::Server { .. } => ServerError::new_err(message),
        Error::Protocol(_) | Error::Decode(_) | Error::Config(_) => {
            SomethingError::new_err(message)
        }
    }
}

/// `frozen` because the facade is immutable after construction; it lets PyO3
/// hand out `&self` without a runtime borrow check, and makes the object safe
/// to share across threads.
#[pyclass(name = "Something", module = "something._something", frozen)]
struct PySomething {
    inner: Arc<Something>,
}

#[pymethods]
impl PySomething {
    /// `mode` overrides `SOMETHING_MODE`; omitting it reads the environment.
    #[new]
    #[pyo3(signature = (mode = None))]
    fn new(py: Python<'_>, mode: Option<&str>) -> PyResult<Self> {
        let mode = match mode {
            Some(value) => value.parse::<Mode>().map_err(to_py_error)?,
            None => Mode::from_env().map_err(to_py_error)?,
        };

        // Construction performs the register round trip, so it can block on a
        // network call just like `ping` does.
        let inner = py
            .detach(|| Something::with_mode(mode))
            .map_err(to_py_error)?;

        Ok(PySomething {
            inner: Arc::new(inner),
        })
    }

    /// Releases the GIL: the call blocks on a Lambda invocation in remote
    /// mode, and holding the GIL through it would stall every other thread.
    fn ping(&self, py: Python<'_>) -> PyResult<String> {
        py.detach(|| self.inner.ping()).map_err(to_py_error)
    }

    #[getter]
    fn negotiated_version(&self) -> u32 {
        self.inner.negotiated_version()
    }

    #[getter]
    fn server_build(&self) -> String {
        self.inner.server_build()
    }

    fn __repr__(&self) -> String {
        format!(
            "Something(transport={}, negotiated_version={}, server_build={})",
            self.inner.transport_description(),
            self.inner.negotiated_version(),
            self.inner.server_build()
        )
    }
}

#[pymodule]
fn _something(module: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = module.py();

    module.add_class::<PySomething>()?;
    module.add("SomethingError", py.get_type::<SomethingError>())?;
    module.add("IncompatibleError", py.get_type::<IncompatibleError>())?;
    module.add("TransportError", py.get_type::<TransportError>())?;
    module.add("ServerError", py.get_type::<ServerError>())?;

    Ok(())
}
