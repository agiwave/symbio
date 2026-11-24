//! Shared runtime host and browser-facing Server library.

mod directories;
mod runtime;
mod server;

pub use runtime::RuntimeHost;
pub use server::{start, ServerConfig, ServerIngress};
pub use symbio_http::stop;
