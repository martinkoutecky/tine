//! Generic physical storage mechanisms shared by Tine persistence domains.

mod filesystem;

pub use filesystem::{
    ensure_directory_nofollow, open_dir_nofollow, open_existing_dir_nofollow, open_file_nofollow,
    publish_immutable_exact, read_optional_regular, read_required_regular, require_regular_entry,
    sync_dir_required, CompletedExactImmutablePublicationBatch, ExactImmutablePublicationBatch,
    FilesystemError, ValidatedDirectorySync,
};
