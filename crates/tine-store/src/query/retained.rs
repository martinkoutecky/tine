//! Conservative retained-memory charging for query-owned serializable data.
//! Count JSON through a sink (no payload allocation), then reserve four bytes
//! per encoded byte plus 128 bytes per array element for container slots.
//! All result fields and IR variants are covered by their existing Serialize
//! implementations. Compiled programs need separate reservations in eval.rs.

use serde::Serialize;
use std::io::{self, Write};

pub(in crate::query) fn parse_config_bytes(config: &tine_core::query::atom::ParseConfig) -> usize {
    serialized_bytes(&(
        &config.separated_by_commas,
        &config.ignored_page_references_keywords,
        &config.hidden_properties,
        &config.journal_page_title_format,
        &config.journal_file_name_format,
    ))
}

pub(in crate::query) fn serialized_bytes(value: &(impl Serialize + ?Sized)) -> usize {
    use serde_json::ser::{CompactFormatter, Formatter};
    let slots = std::cell::Cell::new(0usize);
    struct Reservations<'a>(&'a std::cell::Cell<usize>);
    impl Formatter for Reservations<'_> {
        fn begin_array_value<W: ?Sized + Write>(
            &mut self,
            writer: &mut W,
            first: bool,
        ) -> io::Result<()> {
            // Short strings and tuples encode smaller than their vector slots.
            // Reserve slots separately, including normal vector growth slack.
            self.0.set(self.0.get().saturating_add(128));
            CompactFormatter.begin_array_value(writer, first)
        }
    }
    struct Count(usize);
    impl Write for Count {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0 = self.0.saturating_add(bytes.len());
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut count = Count(0);
    let mut serializer = serde_json::Serializer::with_formatter(&mut count, Reservations(&slots));
    if value.serialize(&mut serializer).is_err() {
        // An unencodable value is still returned to the caller, but cannot be
        // retained under a memory estimate we could not establish.
        return usize::MAX;
    }
    count.0.saturating_mul(4).saturating_add(slots.get())
}

#[cfg(test)]
mod tests {
    #[test]
    fn b_query_short_strings_charge_their_vector_slots() {
        let strings = vec![String::new(); 10_000];
        let actual_slots = strings.capacity() * std::mem::size_of::<String>();
        assert!(
            super::serialized_bytes(&strings) >= actual_slots,
            "I-22: encoded bytes must also charge container slots; imitate query/retained.rs"
        );
    }
}
