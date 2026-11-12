//! Independent native physical coordinates for the frozen merge bridge.
use super::*;

fn quote_ranges(blocks: &[tine_core::lsdoc::ast::Block], out: &mut Vec<(usize, usize)>) {
    use tine_core::lsdoc::ast::{Block, Span};
    for block in blocks {
        match block {
            Block::Quote {
                span: Some(Span(start, end)),
                ..
            } => out.push((*start, *end)),
            Block::Custom { children, .. } => quote_ranges(children, out),
            Block::List { items, .. } => quote_ranges_in_items(items, out),
            _ => {}
        }
    }
}

fn quote_ranges_in_items(items: &[tine_core::lsdoc::ast::ListItem], out: &mut Vec<(usize, usize)>) {
    for item in items {
        quote_ranges(&item.content, out);
        quote_ranges_in_items(&item.items, out);
    }
}

pub(super) fn decorate(file: &mut Value) {
    let text = file["text"].as_str().unwrap();
    let org = file["format"] == "org";
    let view = tine_core::org::lone_cr_to_lf(text);
    let regions = tine_core::block_regions::parse_document(&view, org);
    let mut quotes = Vec::new();
    let parsed = tine_core::render::parse_text_bounded(&view, org).expect("native layout parse");
    quote_ranges(&parsed.blocks, &mut quotes);
    let mut header_end = file["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["kind"] == "header")
        .map_or(0, |b| b["endByte"].as_u64().unwrap() as usize);
    let mut physical = BTreeMap::new();
    let mut at = 0;
    for line in tine_core::doc::physical_lines(text) {
        physical.insert(at, (at, at + line.len()));
        at += line.len();
    }
    let boxes = regions.property_drawer_boundaries(&view);
    let outline = tine_core::refs::physical_outline(text, org);
    let first_root = outline.first().map_or(text.len(), |first| first.0.start);
    // Native container ownership does not disappear when it has no entries.
    // Keep the reader's row classification independently visible to the gate.
    for &(open, close) in &boxes {
        if close < first_root
            && !quotes
                .iter()
                .any(|&(start, end)| start <= open && close < end)
        {
            header_end = header_end.max(physical[&close].1);
        }
    }
    let mut native_properties = regions.properties.clone();
    // A first-root Markdown header has native outline prefixes. Read its
    // dedented physical view, then carry the parser's offsets back by line;
    // a whole-file property span must not guess through the bullet.
    if let Some(first) = outline
        .first()
        .filter(|first| !org && first.0.start == 0 && first.0.end == header_end)
    {
        let mut logical = String::new();
        let mut map = BTreeMap::new();
        for &(start, end) in physical.values().take_while(|(_, end)| *end <= header_end) {
            let skip = if start == 0 {
                first.1
            } else {
                text[start..end]
                    .bytes()
                    .take(first.3)
                    .take_while(|b| matches!(b, b' ' | b'\t'))
                    .count()
            };
            map.insert(logical.len(), start + skip);
            logical.push_str(&view[start + skip..end]);
        }
        native_properties.retain(|p| p.line.0 >= header_end);
        let header = tine_core::block_regions::parse_document(&logical, org);
        for mut property in header.properties {
            let logical_start = property.line.0;
            let physical_start = map[&logical_start];
            let shift = |r: tine_core::block_regions::Range| {
                tine_core::block_regions::Range(
                    physical_start + r.0 - logical_start,
                    physical_start + r.1 - logical_start,
                )
            };
            property.line = shift(property.line);
            property.key_range = shift(property.key_range);
            property.value_range = shift(property.value_range);
            native_properties.push(property);
        }
        native_properties.sort_by_key(|p| p.line.0);
    }
    let mut properties = Vec::new();
    for property in &native_properties {
        if !property.applicable {
            continue;
        }
        let Some((&line_start, &(start, end))) = physical.range(..=property.line.0).next_back()
        else {
            continue;
        };
        let drawer = boxes
            .iter()
            .position(|&(open, close)| open < line_start && line_start < close);
        let key_start = property.key_range.0
            - if property.directive {
                2
            } else if org && drawer.is_some() {
                1
            } else {
                0
            };
        let mut members = Vec::new();
        if tine_core::page_header::is_alias_key(&property.key) {
            let value_start = property.value_range.0;
            let value = &text[property.value_range.0..property.value_range.1];
            tine_core::property_refs::for_each_linkable_member(value, org, |name, token, plain| {
                let Some(token) = token else {
                    return;
                };
                let name_span = if plain {
                    Some(token.clone())
                } else {
                    value[token.clone()]
                        .find(&name)
                        .map(|at| token.start + at..token.start + at + name.len())
                };
                let Some(name_span) = name_span else {
                    return;
                };
                members.push(json!({"name": name, "token": [value_start + token.start, value_start + token.end],
                    "nameSpan": [value_start + name_span.start, value_start + name_span.end]}));
            });
        }
        properties.push((drawer, end <= header_end, json!({"key": property.key.to_ascii_lowercase(),
            "startByte": if org && drawer.is_some() { start } else { key_start }, "endByte": end, "members": members})));
    }
    let drawers: Vec<_> = boxes.iter().enumerate().map(|(index, &(open, close))| {
        let opening = physical[&open];
        let closing = physical[&close];
        json!({"opening": [opening.0, opening.1], "closing": [closing.0, closing.1],
            "page": closing.1 <= header_end,
            "inQuote": quotes.iter().any(|&(start, end)| start <= open && close < end),
            "entries": properties.iter().filter(|(d, _, _)| *d == Some(index)).map(|(_, _, p)| p.clone()).collect::<Vec<_>>()})
    }).collect();
    let directives: Vec<_> = properties
        .iter()
        .filter(|(d, page, _)| d.is_none() && *page)
        .map(|(_, _, p)| p.clone())
        .collect();
    let fields: Vec<_> = properties.iter().filter(|(_, page, p)| *page && tine_core::page_header::is_alias_key(p["key"].as_str().unwrap()))
        .map(|(_, _, p)| {
            let start = p["startByte"].as_u64().unwrap() as usize;
            let end = p["endByte"].as_u64().unwrap() as usize;
            json!({"startByte": start, "endByte": end, "ending": tine_core::doc::line_terminator(&text[start..end]),
                "names": p["members"].as_array().unwrap().iter().map(|m| m["name"].clone()).collect::<Vec<_>>()})
        }).collect();
    file["nativeFields"] = json!(fields);
    file["orgLayout"] =
        json!({"headerEnd": header_end, "drawers": drawers, "directives": directives});
}
