//! Test projection through the same reader as the Wasm query EDN operation.

use super::{forms, instant, parse, read, Edn, Form, Mode, QUERY_BYTES};
use serde_json::{json, Value};

pub fn observe(source: &str, operation: &str) -> String {
    let observed = match operation {
        "observe-first" => parse(source).map(observe_value),
        "observe-stream" => forms(source, Mode::Query).map(|forms| {
            json!([
                "stream",
                forms.into_iter().map(observe_form).collect::<Vec<_>>()
            ])
        }),
        _ => read(source, QUERY_BYTES, Mode::Edn).map(observe_form),
    };
    serde_json::to_string(&observed).unwrap()
}

fn observe_form(form: Form) -> Value {
    observe_value(form.value())
}

fn units(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn observe_value(value: Edn) -> Value {
    match value {
        Edn::Nil => json!(["nil"]),
        Edn::Bool(value) => json!(["bool", value]),
        Edn::Int(value) => observe_number(value as f64),
        Edn::Float(value) => observe_number(value),
        Edn::Str(value) => json!(["string", units(&value)]),
        Edn::Utf16(value) => json!(["string", value]),
        Edn::Keyword(value) => json!(["keyword", value]),
        Edn::Symbol(value) => json!(["symbol", value]),
        Edn::Vec(items) => observe_items("vector", items),
        Edn::Set(items) => observe_items("set", items),
        Edn::List(items) => observe_items("list", items),
        Edn::Map(pairs) => json!([
            "map",
            pairs
                .into_iter()
                .map(|(key, value)| { [observe_value(key), observe_value(value)] })
                .collect::<Vec<_>>()
        ]),
        Edn::Tagged(tag, value) => match (tag.as_str(), *value) {
            ("uuid", value) => json!(["uuid", value.string_units()]),
            ("inst", Edn::Str(value)) => {
                json!(["instant", instant(&value).map(|n| n - 719_163 * 86_400_000)])
            }
            ("queue", Edn::Vec(items)) => observe_items("queue", items),
            ("js", Edn::Vec(items)) => observe_items("js-vector", items),
            ("js", Edn::Map(pairs)) => json!([
                "js-map",
                pairs
                    .into_iter()
                    .map(|(key, value)| { json!([key.string_units(), observe_value(value)]) })
                    .collect::<Vec<_>>()
            ]),
            (_, value) => json!(["tagged", tag, observe_value(value)]),
        },
    }
}

fn observe_items(kind: &str, items: Vec<Edn>) -> Value {
    json!([
        kind,
        items.into_iter().map(observe_value).collect::<Vec<_>>()
    ])
}

fn observe_number(value: f64) -> Value {
    let value = if value.is_nan() { f64::NAN } else { value };
    json!(["number", format!("{:016x}", value.to_bits())])
}
