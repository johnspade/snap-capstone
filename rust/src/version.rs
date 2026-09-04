use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

use serde::de::{self, SeqAccess, Visitor};
use serde::ser::SerializeSeq;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

const MAX_REVISION: u64 = 9_007_199_254_740_991;
const MAX_CONTRIBUTOR_BYTES: usize = 254;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ParseError {
    #[error("invalid version: {0}")]
    InvalidVersion(String),
    #[error("invalid contributor id: {0}")]
    InvalidContributorId(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContributorId(String);

impl ContributorId {
    /// # Errors
    /// Returns `ParseError::InvalidContributorId` if the string is not a valid email-shaped ASCII contributor ID.
    pub fn new(s: &str) -> Result<Self, ParseError> {
        validate_contributor_id(s)?;
        Ok(Self(s.to_owned()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ContributorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn validate_contributor_id(s: &str) -> Result<(), ParseError> {
    let err = || ParseError::InvalidContributorId(s.to_owned());

    if s.is_empty() || s.len() > MAX_CONTRIBUTOR_BYTES {
        return Err(err());
    }

    let at_count = s.bytes().filter(|&b| b == b'@').count();
    if at_count != 1 {
        return Err(err());
    }

    let at_pos = s.find('@').unwrap();
    if at_pos == 0 || at_pos == s.len() - 1 {
        return Err(err());
    }

    for &b in s.as_bytes() {
        if b.is_ascii_control() || b.is_ascii_whitespace() {
            return Err(err());
        }
        if b == b',' || b == b'(' || b == b')' {
            return Err(err());
        }
    }

    if s.contains("->") {
        return Err(err());
    }

    if !s.is_ascii() {
        return Err(err());
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Version {
    components: Vec<(ContributorId, u64)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CausalOrder {
    Equal,
    Before,
    After,
    Concurrent,
}

impl Version {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            components: Vec::new(),
        }
    }

    /// # Errors
    /// Returns `ParseError::InvalidVersion` if the components contain duplicates, zero revisions, or overflow.
    pub fn new(mut components: Vec<(ContributorId, u64)>) -> Result<Self, ParseError> {
        components.sort_by(|(a, _), (b, _)| a.cmp(b));

        for i in 1..components.len() {
            if components[i].0 == components[i - 1].0 {
                return Err(ParseError::InvalidVersion(format!(
                    "duplicate contributor: {}",
                    components[i].0
                )));
            }
        }

        for (_, rev) in &components {
            if *rev == 0 {
                return Err(ParseError::InvalidVersion(
                    "explicit zero revision".to_owned(),
                ));
            }
            if *rev > MAX_REVISION {
                return Err(ParseError::InvalidVersion("revision overflow".to_owned()));
            }
        }

        Ok(Self { components })
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.components.is_empty()
    }

    #[must_use]
    pub fn components(&self) -> &[(ContributorId, u64)] {
        &self.components
    }

    #[must_use]
    pub fn get(&self, id: &ContributorId) -> u64 {
        self.components
            .iter()
            .find(|(c, _)| c == id)
            .map_or(0, |(_, r)| *r)
    }

    #[must_use]
    pub fn compare_causal(&self, other: &Self) -> CausalOrder {
        let all_ids = self.all_ids_union(other);

        let mut has_less = false;
        let mut has_greater = false;

        for id in &all_ids {
            let a = self.get(id);
            let b = other.get(id);
            match a.cmp(&b) {
                Ordering::Less => has_less = true,
                Ordering::Greater => has_greater = true,
                Ordering::Equal => {}
            }
            if has_less && has_greater {
                return CausalOrder::Concurrent;
            }
        }

        match (has_less, has_greater) {
            (false, false) => CausalOrder::Equal,
            (true, false) => CausalOrder::Before,
            (false, true) => CausalOrder::After,
            (true, true) => CausalOrder::Concurrent,
        }
    }

    #[must_use]
    pub fn join(&self, other: &Self) -> Self {
        let all_ids = self.all_ids_union(other);
        let components = all_ids
            .into_iter()
            .map(|id| {
                let rev = self.get(&id).max(other.get(&id));
                (id, rev)
            })
            .filter(|(_, rev)| *rev > 0)
            .collect();
        Self { components }
    }

    #[must_use]
    pub fn snap_cmp(&self, other: &Self) -> Ordering {
        let all_ids = self.all_ids_union(other);
        for id in &all_ids {
            let a = self.get(id);
            let b = other.get(id);
            let ord = a.cmp(&b);
            if ord != Ordering::Equal {
                return ord;
            }
        }
        Ordering::Equal
    }

    fn all_ids_union(&self, other: &Self) -> Vec<ContributorId> {
        let mut ids: Vec<ContributorId> = self
            .components
            .iter()
            .map(|(id, _)| id.clone())
            .chain(other.components.iter().map(|(id, _)| id.clone()))
            .collect();
        ids.sort();
        ids.dedup();
        ids
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(")?;
        for (i, (id, rev)) in self.components.iter().enumerate() {
            if i > 0 {
                f.write_str(",")?;
            }
            write!(f, "{id}->{rev}")?;
        }
        f.write_str(")")
    }
}

impl FromStr for Version {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || ParseError::InvalidVersion(s.to_owned());

        if s.contains(char::is_whitespace) {
            return Err(err());
        }

        if !s.starts_with('(') || !s.ends_with(')') {
            return Err(err());
        }

        let inner = &s[1..s.len() - 1];

        if inner.is_empty() {
            return Ok(Self::empty());
        }

        let parts: Vec<&str> = inner.split(',').collect();
        let mut components = Vec::with_capacity(parts.len());

        for part in parts {
            let arrow = part.find("->").ok_or_else(err)?;
            let id_str = &part[..arrow];
            let rev_str = &part[arrow + 2..];

            if rev_str.is_empty() {
                return Err(err());
            }

            if rev_str.len() > 1 && rev_str.starts_with('0') {
                return Err(err());
            }

            let rev: u64 = rev_str.parse().map_err(|_| err())?;
            let id = ContributorId::new(id_str)?;

            components.push((id, rev));
        }

        let is_sorted = components.windows(2).all(|w| w[0].0 < w[1].0);
        if !is_sorted {
            return Err(err());
        }

        Self::new(components)
    }
}

impl Serialize for Version {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(self.components.len()))?;
        for (id, rev) in &self.components {
            seq.serialize_element(&(id.as_str(), rev))?;
        }
        seq.end()
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct VersionVisitor;

        impl<'de> Visitor<'de> for VersionVisitor {
            type Value = Version;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an array of [id, revision] pairs")
            }

            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut components = Vec::new();
                while let Some((id_str, rev)) = seq.next_element::<(String, u64)>()? {
                    let id = ContributorId::new(&id_str).map_err(de::Error::custom)?;
                    if let Some((prev_id, _)) = components.last() {
                        if &id <= prev_id {
                            return Err(de::Error::custom(
                                "version components are not in canonical order",
                            ));
                        }
                    }
                    components.push((id, rev));
                }
                Version::new(components).map_err(de::Error::custom)
            }
        }

        deserializer.deserialize_seq(VersionVisitor)
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(std::cmp::Ord::cmp(self, other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.snap_cmp(other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_version_parses_and_displays() {
        let v: Version = "()".parse().unwrap();
        assert!(v.is_empty());
        assert_eq!(v.to_string(), "()");
    }

    #[test]
    fn single_component_round_trip() {
        let v: Version = "(alice@example.com->42)".parse().unwrap();
        assert_eq!(v.to_string(), "(alice@example.com->42)");
    }

    #[test]
    fn multi_component_round_trip() {
        let s = "(a@x->1,b@y->2,c@z->3)";
        let v: Version = s.parse().unwrap();
        assert_eq!(v.to_string(), s);
    }

    #[test]
    fn rejects_leading_zeroes() {
        assert!("(a@x->01)".parse::<Version>().is_err());
    }

    #[test]
    fn rejects_explicit_zero() {
        assert!("(a@x->0)".parse::<Version>().is_err());
    }

    #[test]
    fn rejects_duplicate_ids() {
        assert!("(a@x->1,a@x->2)".parse::<Version>().is_err());
    }

    #[test]
    fn rejects_noncanonical_ordering() {
        assert!("(b@x->1,a@x->2)".parse::<Version>().is_err());
    }

    #[test]
    fn rejects_whitespace() {
        assert!("(a@x->1, b@x->2)".parse::<Version>().is_err());
        assert!("( a@x->1)".parse::<Version>().is_err());
        assert!(" (a@x->1)".parse::<Version>().is_err());
        assert!("(a@x->1) ".parse::<Version>().is_err());
    }

    #[test]
    fn rejects_overflow() {
        assert!("(a@x->9007199254740992)".parse::<Version>().is_err());
    }

    #[test]
    fn accepts_max_revision() {
        let v: Version = "(a@x->9007199254740991)".parse().unwrap();
        assert_eq!(v.get(&ContributorId::new("a@x").unwrap()), MAX_REVISION);
    }

    #[test]
    fn rejects_missing_parens() {
        assert!("a@x->1".parse::<Version>().is_err());
    }

    #[test]
    fn rejects_empty_string() {
        assert!("".parse::<Version>().is_err());
    }

    #[test]
    fn contributor_id_valid() {
        assert!(ContributorId::new("alice@example.com").is_ok());
        assert!(ContributorId::new("a@x").is_ok());
    }

    #[test]
    fn contributor_id_rejects_no_at() {
        assert!(ContributorId::new("alice").is_err());
    }

    #[test]
    fn contributor_id_rejects_multiple_at() {
        assert!(ContributorId::new("a@b@c").is_err());
    }

    #[test]
    fn contributor_id_rejects_empty_local() {
        assert!(ContributorId::new("@example.com").is_err());
    }

    #[test]
    fn contributor_id_rejects_empty_domain() {
        assert!(ContributorId::new("alice@").is_err());
    }

    #[test]
    fn contributor_id_rejects_control_chars() {
        assert!(ContributorId::new("a\x00@x").is_err());
        assert!(ContributorId::new("a\n@x").is_err());
    }

    #[test]
    fn contributor_id_rejects_whitespace() {
        assert!(ContributorId::new("a @x").is_err());
        assert!(ContributorId::new("a\t@x").is_err());
    }

    #[test]
    fn contributor_id_rejects_forbidden_chars() {
        assert!(ContributorId::new("a,b@x").is_err());
        assert!(ContributorId::new("a(b@x").is_err());
        assert!(ContributorId::new("a)b@x").is_err());
    }

    #[test]
    fn contributor_id_rejects_arrow_substring() {
        assert!(ContributorId::new("a->b@x").is_err());
    }

    #[test]
    fn contributor_id_rejects_too_long() {
        let long = format!("{}@x", "a".repeat(253));
        assert!(ContributorId::new(&long).is_err());
    }

    #[test]
    fn contributor_id_accepts_max_length() {
        let id = format!("{}@x", "a".repeat(252));
        assert_eq!(id.len(), 254);
        assert!(ContributorId::new(&id).is_ok());
    }

    #[test]
    fn contributor_id_rejects_non_ascii() {
        assert!(ContributorId::new("ä@x").is_err());
    }

    #[test]
    fn causal_equal() {
        let v: Version = "(a@x->1,b@y->2)".parse().unwrap();
        let w: Version = "(a@x->1,b@y->2)".parse().unwrap();
        assert_eq!(v.compare_causal(&w), CausalOrder::Equal);
    }

    #[test]
    fn causal_before() {
        let v: Version = "(a@x->1)".parse().unwrap();
        let w: Version = "(a@x->1,b@y->1)".parse().unwrap();
        assert_eq!(v.compare_causal(&w), CausalOrder::Before);
    }

    #[test]
    fn causal_after() {
        let v: Version = "(a@x->2,b@y->1)".parse().unwrap();
        let w: Version = "(a@x->1,b@y->1)".parse().unwrap();
        assert_eq!(v.compare_causal(&w), CausalOrder::After);
    }

    #[test]
    fn causal_concurrent() {
        let v: Version = "(a@x->2)".parse().unwrap();
        let w: Version = "(b@y->1)".parse().unwrap();
        assert_eq!(v.compare_causal(&w), CausalOrder::Concurrent);
    }

    #[test]
    fn causal_concurrent_overlapping() {
        let v: Version = "(a@x->2,b@y->1)".parse().unwrap();
        let w: Version = "(a@x->1,b@y->2)".parse().unwrap();
        assert_eq!(v.compare_causal(&w), CausalOrder::Concurrent);
    }

    #[test]
    fn causal_empty_vs_empty() {
        assert_eq!(
            Version::empty().compare_causal(&Version::empty()),
            CausalOrder::Equal
        );
    }

    #[test]
    fn causal_empty_before_nonempty() {
        let v = Version::empty();
        let w: Version = "(a@x->1)".parse().unwrap();
        assert_eq!(v.compare_causal(&w), CausalOrder::Before);
    }

    #[test]
    fn join_basic() {
        let v: Version = "(a@x->2)".parse().unwrap();
        let w: Version = "(a@x->1,b@y->3)".parse().unwrap();
        let j = v.join(&w);
        assert_eq!(j.to_string(), "(a@x->2,b@y->3)");
    }

    #[test]
    fn join_disjoint() {
        let v: Version = "(a@x->1)".parse().unwrap();
        let w: Version = "(b@y->2)".parse().unwrap();
        let j = v.join(&w);
        assert_eq!(j.to_string(), "(a@x->1,b@y->2)");
    }

    #[test]
    fn join_empty() {
        let v = Version::empty();
        let w: Version = "(a@x->1)".parse().unwrap();
        assert_eq!(v.join(&w), w);
        assert_eq!(w.join(&v), w);
    }

    #[test]
    fn snap_order_extends_causal() {
        let v: Version = "(a@x->1)".parse().unwrap();
        let w: Version = "(a@x->2)".parse().unwrap();
        assert_eq!(v.snap_cmp(&w), Ordering::Less);
        assert_eq!(w.snap_cmp(&v), Ordering::Greater);
    }

    #[test]
    fn snap_order_concurrent() {
        let v: Version = "(a@x->2)".parse().unwrap();
        let w: Version = "(b@y->1)".parse().unwrap();
        // a@x has rev 2 vs 0 for w, so v > w in snap order
        assert_eq!(v.snap_cmp(&w), Ordering::Greater);
    }

    #[test]
    fn snap_order_equal() {
        let v: Version = "(a@x->1)".parse().unwrap();
        let w: Version = "(a@x->1)".parse().unwrap();
        assert_eq!(v.snap_cmp(&w), Ordering::Equal);
    }

    #[test]
    fn json_round_trip() {
        let v: Version = "(a@x->1,b@y->2)".parse().unwrap();
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, r#"[["a@x",1],["b@y",2]]"#);
        let w: Version = serde_json::from_str(&json).unwrap();
        assert_eq!(v, w);
    }

    #[test]
    fn json_empty() {
        let v = Version::empty();
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, "[]");
        let w: Version = serde_json::from_str(&json).unwrap();
        assert_eq!(v, w);
    }

    #[test]
    fn json_rejects_duplicate_ids() {
        let json = r#"[["a@x",1],["a@x",2]]"#;
        assert!(serde_json::from_str::<Version>(json).is_err());
    }

    #[test]
    fn json_rejects_zero_revision() {
        let json = r#"[["a@x",0]]"#;
        assert!(serde_json::from_str::<Version>(json).is_err());
    }
}

#[cfg(all(test, not(miri)))]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    fn arb_contributor_id() -> impl Strategy<Value = ContributorId> {
        "[a-z]{1,8}@[a-z]{1,8}\\.[a-z]{2,4}"
            .prop_filter_map("valid contributor id", |s| ContributorId::new(&s).ok())
    }

    fn arb_revision() -> impl Strategy<Value = u64> {
        1..=10_000u64
    }

    fn arb_version() -> impl Strategy<Value = Version> {
        prop::collection::vec((arb_contributor_id(), arb_revision()), 0..=5).prop_filter_map(
            "valid version",
            |mut comps| {
                comps.sort_by(|(a, _), (b, _)| a.cmp(b));
                comps.dedup_by(|(a, _), (b, _)| a == b);
                Version::new(comps).ok()
            },
        )
    }

    proptest! {
        #[test]
        fn parse_display_round_trip(v in arb_version()) {
            let s = v.to_string();
            let w: Version = s.parse().unwrap();
            prop_assert_eq!(v, w);
        }

        #[test]
        fn json_round_trip(v in arb_version()) {
            let json = serde_json::to_string(&v).unwrap();
            let w: Version = serde_json::from_str(&json).unwrap();
            prop_assert_eq!(v, w);
        }

        #[test]
        fn join_commutative(v in arb_version(), w in arb_version()) {
            prop_assert_eq!(v.join(&w), w.join(&v));
        }

        #[test]
        fn join_associative(a in arb_version(), b in arb_version(), c in arb_version()) {
            prop_assert_eq!(a.join(&b).join(&c), a.join(&b.join(&c)));
        }

        #[test]
        fn join_idempotent(v in arb_version()) {
            prop_assert_eq!(v.join(&v), v);
        }

        #[test]
        fn snap_order_extends_causal(v in arb_version(), w in arb_version()) {
            match v.compare_causal(&w) {
                CausalOrder::Before => prop_assert_eq!(v.snap_cmp(&w), std::cmp::Ordering::Less),
                CausalOrder::After => prop_assert_eq!(v.snap_cmp(&w), std::cmp::Ordering::Greater),
                CausalOrder::Equal => prop_assert_eq!(v.snap_cmp(&w), std::cmp::Ordering::Equal),
                CausalOrder::Concurrent => {
                    // Snap order gives some total order, just not Equal
                    prop_assert_ne!(v.snap_cmp(&w), std::cmp::Ordering::Equal);
                }
            }
        }

        #[test]
        fn snap_order_total(v in arb_version(), w in arb_version()) {
            let vw = v.snap_cmp(&w);
            let wv = w.snap_cmp(&v);
            prop_assert_eq!(vw, wv.reverse());
        }
    }
}
