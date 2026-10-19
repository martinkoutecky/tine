//! Matchers for the `((` block picker's two OG legs (`electron/search.cljs`
//! `search-blocks`), both reading the folded text of one scan.
//!
//! Fragments: OG's `content LIKE '%a%b%'`; every whitespace-separated piece of
//! the folded query occurs literally in the text, in order, without overlap
//! (greedy leftmost matching decides membership exactly).
//!
//! Words: OG's FTS5 `content MATCH ?`. A predicate value holds the word groups
//! encoded as one group per line, the tokens of a group separated by single
//! spaces.
//!
//! FTS5 semantics reproduced: a token is a maximal run of letters, digits and
//! combining marks; a query word matches a whole token only (no prefix, no
//! substring); several words are ANDed in any order; one query word that
//! tokenizes into several tokens is a phrase (adjacent tokens, in order). A
//! query piece containing any other ASCII character (`-`, `'`, `.`, `:`, ...)
//! makes OG's unquoted MATCH a syntax error, so only its quoted-phrase input
//! (all tokens of the query, adjacent, in order) runs. Differences: no
//! `and`/`or`/`not`/`&`/`|` rewriting, no trailing-`*` prefix queries, no bm25
//! order (see `TextMatchMode::Words`).

fn is_token_char(c: char) -> bool {
    c.is_alphanumeric() || ('\u{0300}'..='\u{036f}').contains(&c)
}

fn tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c| !is_token_char(c)).filter(|t| !t.is_empty())
}

/// What FTS5 accepts as a bare word: ASCII alphanumerics, `_` and any
/// non-ASCII character.
fn is_bareword(piece: &str) -> bool {
    piece
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || !c.is_ascii())
}

/// The encoded word groups for an already-folded picker query, or `None` when
/// the full-text leg has nothing to match (blank query, or no tokens at all).
pub(super) fn words_value(folded_query: &str) -> Option<String> {
    let join = |text: &str| tokens(text).collect::<Vec<_>>().join(" ");
    let groups: Vec<String> = if folded_query.split_whitespace().all(is_bareword) {
        folded_query
            .split_whitespace()
            .map(join)
            .filter(|group| !group.is_empty())
            .collect()
    } else {
        vec![join(folded_query)]
    };
    let groups: Vec<String> = groups.into_iter().filter(|g| !g.is_empty()).collect();
    (!groups.is_empty()).then(|| groups.join("\n"))
}

/// One word group: a phrase of whole tokens. The first token's searcher is built
/// once per plan, because setting up a substring searcher per block costs as
/// much as the search itself on a graph of hundreds of thousands of blocks.
#[derive(Debug, Clone)]
struct WordGroup {
    first: memchr::memmem::Finder<'static>,
    first_len: usize,
    rest: Vec<String>,
}

impl WordGroup {
    fn new(group: &str) -> Option<Self> {
        let mut words = group.split(' ');
        let first = words.next().filter(|first| !first.is_empty())?;
        Some(Self {
            first: memchr::memmem::Finder::new(first).into_owned(),
            first_len: first.len(),
            rest: words.map(str::to_owned).collect(),
        })
    }

    /// Leftmost whole-token occurrence of the phrase in the folded `lower`, as
    /// a byte range.
    fn find(&self, lower: &str) -> Option<(usize, usize)> {
        'candidates: for start in self.first.find_iter(lower.as_bytes()) {
            if lower[..start]
                .chars()
                .next_back()
                .is_some_and(is_token_char)
            {
                continue;
            }
            let mut end = start + self.first_len;
            if lower[end..].chars().next().is_some_and(is_token_char) {
                continue;
            }
            for word in &self.rest {
                let tail = &lower[end..];
                let gap = tail.len() - tail.trim_start_matches(|c| !is_token_char(c)).len();
                let begin = end + gap;
                if !lower[begin..].starts_with(word.as_str()) {
                    continue 'candidates;
                }
                end = begin + word.len();
                if lower[end..].chars().next().is_some_and(is_token_char) {
                    continue 'candidates;
                }
            }
            return Some((start, end));
        }
        None
    }
}

/// A compiled `TextMatchMode::Words` value.
#[derive(Debug, Clone)]
pub(super) struct WordsMatcher {
    groups: Vec<WordGroup>,
}

impl WordsMatcher {
    pub(super) fn new(value: &str) -> Self {
        Self {
            groups: value.split('\n').filter_map(WordGroup::new).collect(),
        }
    }

    /// Byte range of each group's leftmost occurrence, or `None` unless every
    /// group occurs (an empty matcher matches nothing).
    pub(super) fn ranges(&self, lower: &str) -> Option<Vec<(usize, usize)>> {
        if self.groups.is_empty() {
            return None;
        }
        self.groups.iter().map(|group| group.find(lower)).collect()
    }

    /// Membership plus the leftmost match offset, without allocating.
    pub(super) fn first_offset(&self, lower: &str) -> Option<usize> {
        if self.groups.is_empty() {
            return None;
        }
        let mut first = usize::MAX;
        for group in &self.groups {
            first = first.min(group.find(lower)?.0);
        }
        Some(first)
    }
}

/// A compiled `TextMatchMode::Fragments` value: OG's `LIKE '%a%b%'` over the
/// whitespace-separated pieces, each searched by a prebuilt searcher (greedy
/// leftmost matching decides membership exactly).
#[derive(Debug, Clone)]
pub(super) struct FragmentsMatcher {
    pieces: Vec<memchr::memmem::Finder<'static>>,
}

impl FragmentsMatcher {
    pub(super) fn new(query: &str) -> Self {
        Self {
            pieces: query
                .split_whitespace()
                .map(|piece| memchr::memmem::Finder::new(piece).into_owned())
                .collect(),
        }
    }

    /// Byte offset of the first piece's leftmost occurrence when every piece
    /// follows in order without overlap; a query with no piece matches nothing.
    pub(super) fn first_offset(&self, lower: &str) -> Option<usize> {
        let mut at = 0;
        let mut first = None;
        for piece in &self.pieces {
            let found = at + piece.find(&lower.as_bytes()[at..])?;
            first.get_or_insert(found);
            at = found + piece.needle().len();
        }
        first
    }
}
