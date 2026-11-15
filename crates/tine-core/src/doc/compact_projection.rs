//! Compact resident projection with the existing flat checkpoint field list.
use super::*;

/// Parsed values derived from a block's current raw text.
// Memoization avoids reparsing each block during whole-graph scans.
//
// Memory shape (GH #623): this slot is paid inline by every block, so it holds only what
// every block has. Facets set on a minority of blocks live in one `Option<Box<..>>`, and
// text that equals another stored text is not stored again: `visible` is `None` when it
// equals the block's `raw`, `visible_lower` is `None` when it equals `visible`. Those
// fallbacks need the block's `raw`, so the ONLY readers are `DocBlock::visible_text` and
// `DocBlock::visible_folded`; the fields are private so no caller can re-implement the
// fallback. Unrelated to behaviour: every accessor returns what the eager form returned.
#[deny(missing_docs)]
//
// Serde keeps the flat field list below (`ProjectionWire`), so checkpoint bytes do not
// depend on this in-memory shape.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(from = "ProjectionWire")]
pub struct BlockProjection {
    /// Parser-owned raw byte regions from the same cached single-block AST.
    /// Sparse edit data stays out of the inline block; empty regions are shared.
    pub regions: std::sync::Arc<crate::block_regions::BlockRegions>,
    /// The stored visible texts; `None` when both equal their fallbacks.
    pub(super) texts: Option<Box<VisibleTexts>>,
    /// Lazy accent-sensitive fold, populated only when that graph policy is used;
    /// the inner `None` means identical to the visible text. Checkpointed in
    /// whatever state it is in (built or not), like every lazy answer. Boxed twice
    /// so the slot takes one word here: that policy is off by default.
    pub(super) visible_literal: std::sync::OnceLock<Option<Box<Box<str>>>>,
    /// Byte ranges of `raw` eligible for plain-text (unlinked) reference matching.
    pub(super) plain_ranges: PlainRanges,
    /// Facets set on a minority of blocks; `None` when the block has none of them.
    pub(super) extra: Option<Box<ProjectionExtra>>,
}

/// The visible texts of a block whose text differs from its raw body.
#[derive(Debug, Clone, Default)]
pub(super) struct VisibleTexts {
    /// Visible (non-property) text, original case — `raw` minus the byte ranges lsdoc
    /// recognized as `Properties` blocks. `None` when identical to `raw`.
    pub(super) visible: Option<Box<str>>,
    /// `visible` folded with accent-removing `canonical_fold`, independent of graph
    /// policy. `None` when identical to the visible text.
    pub(super) visible_lower: Option<Box<str>>,
}

/// Plain-search ranges, with no allocation for the usual one range.
#[derive(Debug, Clone, Default)]
pub(super) enum PlainRanges {
    #[default]
    None,
    One(u32, u32),
    Many(Box<Box<[std::ops::Range<usize>]>>),
}

impl PlainRanges {
    pub(super) fn view(&self) -> crate::reference_evidence::PlainRangeList<'_> {
        use crate::reference_evidence::PlainRangeList;
        match self {
            Self::None => PlainRangeList::Many(&[]),
            Self::One(start, end) => PlainRangeList::One(*start, *end),
            Self::Many(ranges) => PlainRangeList::Many(ranges),
        }
    }
}

impl From<Vec<std::ops::Range<usize>>> for PlainRanges {
    fn from(ranges: Vec<std::ops::Range<usize>>) -> Self {
        match ranges.as_slice() {
            [] => Self::None,
            [one] => match (u32::try_from(one.start), u32::try_from(one.end)) {
                (Ok(start), Ok(end)) => Self::One(start, end),
                _ => Self::Many(Box::new(ranges.into_boxed_slice())),
            },
            _ => Self::Many(Box::new(ranges.into_boxed_slice())),
        }
    }
}

/// [`BlockProjection`]'s serde shape: every stored answer as a flat field.
#[derive(Deserialize)]
struct ProjectionWire {
    #[serde(deserialize_with = "deserialize_shared_regions")]
    regions: std::sync::Arc<crate::block_regions::BlockRegions>,
    visible: Option<Box<str>>,
    visible_lower: Option<Box<str>>,
    visible_literal: Option<Option<Box<str>>>,
    plain_ranges: Vec<std::ops::Range<usize>>,
    extra: Option<Box<ProjectionExtra>>,
}

impl From<ProjectionWire> for BlockProjection {
    fn from(wire: ProjectionWire) -> Self {
        let visible_literal = std::sync::OnceLock::new();
        if let Some(literal) = wire.visible_literal {
            let _ = visible_literal.set(literal.map(Box::new));
        }
        BlockProjection {
            regions: wire.regions,
            texts: VisibleTexts::boxed(wire.visible, wire.visible_lower),
            visible_literal,
            plain_ranges: wire.plain_ranges.into(),
            extra: wire.extra,
        }
    }
}

impl Serialize for BlockProjection {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        struct Ranges<'a>(&'a PlainRanges);
        impl Serialize for Ranges<'_> {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_seq(self.0.view().iter())
            }
        }
        let texts = self.texts.as_deref();
        let mut out = s.serialize_struct("BlockProjection", 6)?;
        out.serialize_field("regions", &*self.regions)?;
        out.serialize_field("visible", &texts.and_then(|t| t.visible.as_deref()))?;
        out.serialize_field(
            "visible_lower",
            &texts.and_then(|t| t.visible_lower.as_deref()),
        )?;
        out.serialize_field(
            "visible_literal",
            &self
                .visible_literal
                .get()
                .map(|literal| literal.as_ref().map(|text| &***text)),
        )?;
        out.serialize_field("plain_ranges", &Ranges(&self.plain_ranges))?;
        out.serialize_field("extra", &self.extra)?;
        out.end()
    }
}

impl VisibleTexts {
    pub(super) fn boxed(
        visible: Option<Box<str>>,
        visible_lower: Option<Box<str>>,
    ) -> Option<Box<Self>> {
        (visible.is_some() || visible_lower.is_some()).then(|| {
            Box::new(Self {
                visible,
                visible_lower,
            })
        })
    }
}
