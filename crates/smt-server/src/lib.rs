//! Reference server-side implementation for the SMT wire protocol.
//!
//! The crate provides a small TCP server, binary request handling, native Z3,
//! binbit, and Bitwuzla solver backends, backend racing, request/response
//! caching, and a compact SMT-LIB frontend.

pub mod backend;
pub mod binbit_backend;
pub mod bitwuzla_backend;
pub mod bitwuzla_bindings;
pub mod cache;
pub mod command_router;
pub mod pool;
pub mod protocol;
pub mod qfbvsmtrs_backend;
pub mod racing;
pub(crate) mod recording;
pub mod rumba_backend;
pub mod server;
pub mod smt2;
pub mod smtlib;
pub mod z3_backend;

pub use backend::{Backend, CancellationToken, QueryResult, QueryStatus, SolveContext};
pub use binbit_backend::BinbitBackend;
pub use bitwuzla_backend::BitwuzlaBackend;
pub use cache::{cache_key_for_payload, rebind_cached_response, CacheStats, ResponseCache};
pub use command_router::CommandRouterBackend;
pub use pool::PooledBackend;
pub use protocol::{handle_binary_frame, handle_binary_request, response_from_query_result};
pub use qfbvsmtrs_backend::QfbvsmtrsBackend;
pub use racing::RacingBackend;
pub use recording::{
    default_legacy_recording_tree, default_recording_db_path,
    migrate_default as migrate_recordings, migrate_tree as migrate_recording_tree,
    recording_db_path, MigrationReport,
};
pub use rumba_backend::RumbaBackend;
pub use server::{dispatch_payload, dispatch_payload_with_cache, serve_tcp, ServerConfig};
pub use smt2::{request_to_smt2, Smt2Script, Smt2Variable};
pub use smtlib::{handle_text_frame, parse_smtlib_script, TextQuery, WireSmtLibSink};
pub use z3_backend::Z3Backend;
