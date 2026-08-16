//! The agent face. Thin JSON-RPC glue over the core library.
//!
//! Protocol `2024-11-05`, newline-delimited JSON over stdio. stdout is the
//! JSON-RPC stream — all tracing goes to stderr.

pub mod dispatch;
pub mod handlers;
pub mod tools;
pub mod transport;
