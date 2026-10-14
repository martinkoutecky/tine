//! Ephemeral, bounded page-name expansions. No dictionary enters the wasm parser.
use pinyin::ToPinyin;

#[cfg(test)]
thread_local! { pub(crate) static BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

#[derive(Debug, Clone, Copy)]
enum Adapter {
    Mandarin,
}

#[derive(Debug)]
pub(crate) struct NameKeys {
    full: String,
    initials: String,
}

fn contains_han(name: &str) -> bool {
    !name.is_ascii()
        && name.chars().any(|c| {
            matches!(c as u32,
            0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff |
            0x20000..=0x323af)
        })
}

pub(crate) fn eligible(name: &str) -> bool {
    Adapter::Mandarin.accepts(name)
}

pub(crate) fn cache_identity(name: &str) -> Option<String> {
    eligible(name).then(|| tine_core::refs::page_key(name))
}

impl Adapter {
    fn accepts(self, name: &str) -> bool {
        match self {
            Self::Mandarin => contains_han(name),
        }
    }
    fn keys(self, name: &str) -> NameKeys {
        let mut full = String::new();
        let mut initials = String::new();
        for c in name.chars() {
            if let Some(reading) = c.to_pinyin() {
                let syllable = reading.plain();
                full.push_str(syllable);
                initials.push(syllable.chars().next().unwrap());
            } else if !c.is_whitespace() && c != '-' && c != '\'' {
                for lower in c.to_lowercase() {
                    full.push(lower);
                    initials.push(lower);
                }
            }
        }
        NameKeys { full, initials }
    }
}

pub(crate) fn normalize_query(query: &str) -> Option<String> {
    let query: String = query
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '-' && *c != '\'')
        .collect();
    (!query.is_empty() && query.bytes().all(|c| c.is_ascii_alphabetic()))
        .then(|| query.to_ascii_lowercase())
}

impl NameKeys {
    pub(crate) fn build(name: &str) -> Self {
        #[cfg(test)]
        BUILDS.with(|count| count.set(count.get() + 1));
        Adapter::Mandarin.keys(name)
    }

    #[cfg(feature = "test-faults")]
    pub(crate) fn allocated_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.full.capacity() + self.initials.capacity()
    }

    pub(crate) fn score(&self, query: &str) -> Option<i32> {
        [&self.full, &self.initials]
            .into_iter()
            .filter_map(|key| {
                if key.starts_with(query) {
                    Some(200)
                } else if key.contains(query) {
                    Some(100)
                } else {
                    None
                }
            })
            .max()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multilingual_default_readings_are_bounded_and_contiguous() {
        for (name, full, initials) in [
            ("北京", "beijing", "bj"),
            ("臺灣", "taiwan", "tw"),
            ("北京Notes", "beijingnotes", "bjnotes"),
            ("東京", "dongjing", "dj"),
        ] {
            let keys = NameKeys::build(name);
            assert_eq!(keys.full, full);
            assert_eq!(keys.initials, initials);
            assert!(keys.score(full).is_some());
            assert!(keys.score(initials).is_some());
        }
        let keys = NameKeys::build("北京");
        assert!(keys.score(&normalize_query("bei jing").unwrap()).is_some());
        assert!(keys.score("bjn").is_none());
        assert!(keys.score("北京").is_none());
        assert!(!contains_han("Notes café"));
        assert_eq!(NameKeys::build("重慶").full, "zhongqing");
    }
}
