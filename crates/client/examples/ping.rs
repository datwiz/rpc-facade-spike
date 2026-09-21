//! Same code, either mode.
//!
//! ```text
//! cargo run --example ping                       # local, in-process
//! SOMETHING_MODE=remote \
//! SOMETHING_LAMBDA_FUNCTION=something-poc \
//! SOMETHING_LAMBDA_QUALIFIER=live \
//!   cargo run --example ping                     # remote, on AWS
//! ```

use something_client::{DoSomething, Error, Something};

/// Takes the trait, not the struct: this function cannot observe the mode.
fn greet(backend: &dyn DoSomething) -> Result<String, Error> {
    backend.ping()
}

fn main() -> Result<(), Error> {
    let client = Something::from_env()?;
    println!("{client:?}");
    println!("{}", greet(&client)?);

    Ok(())
}
