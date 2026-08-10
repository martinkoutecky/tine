use tine_storage::formats::{FormatKind, FormatValue, FORMAT_MANIFEST};

fn kind_name(kind: FormatKind) -> &'static str {
    match kind {
        FormatKind::Identity => "identity",
        FormatKind::Layout => "layout",
        FormatKind::WriterBound => "writer-bound",
        FormatKind::CheckpointGeometry => "checkpoint-geometry",
    }
}

fn main() {
    for row in FORMAT_MANIFEST {
        let value = match row.value {
            FormatValue::Number(value) => value.to_string(),
            FormatValue::Name(value) => value.to_owned(),
        };
        println!(
            "{}\t{}\t{}\t{}",
            row.name,
            kind_name(row.kind),
            row.artifact,
            value
        );
    }
}
