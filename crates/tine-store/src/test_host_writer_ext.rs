//! The host-writer fixture for test modules written against the dev-dependency
//! build of this crate, the instance tine-graph-features also links.
use tine_store as store_crate;
#[path = "test_host_writer.rs"]
mod host_writer;
pub(crate) use host_writer::*;
