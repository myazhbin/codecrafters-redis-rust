//! Sorted sets: unique members, each with a floating-point score, kept in order of score.
//! Members with the same score are in byte order.

use std::cmp::Ordering;
use std::collections::HashMap;

#[derive(Default)]
pub struct SortedSet {
    /// Each member's score, to look it up quickly.
    scores: HashMap<Vec<u8>, f64>,
    /// The members with their scores, in order, to find ranks and ranges by binary search.
    order: Vec<(f64, Vec<u8>)>,
}

/// The order of members in a sorted set: by score, then by bytes. Scores are never NaN.
fn compare(a: (f64, &[u8]), b: (f64, &[u8])) -> Ordering {
    a.0.partial_cmp(&b.0)
        .unwrap_or(Ordering::Equal)
        .then_with(|| a.1.cmp(b.1))
}

impl SortedSet {
    pub fn len(&self) -> usize {
        self.order.len()
    }

    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// Where a member with `score` is, or would go, in `order`.
    fn position(&self, score: f64, member: &[u8]) -> Result<usize, usize> {
        self.order
            .binary_search_by(|(s, m)| compare((*s, m), (score, member)))
    }

    /// Adds `member` with `score`, or moves it to `score` if it's already in the set.
    /// Returns whether it was new.
    pub fn insert(&mut self, member: &[u8], score: f64) -> bool {
        let new = match self.scores.insert(member.to_vec(), score) {
            Some(old) => {
                let i = self.position(old, member).expect("a member is where its score says");
                self.order.remove(i);
                false
            }
            None => true,
        };
        let i = self.position(score, member).unwrap_or_else(|i| i);
        self.order.insert(i, (score, member.to_vec()));
        new
    }

    /// Removes `member`, returning whether it was in the set.
    pub fn remove(&mut self, member: &[u8]) -> bool {
        let Some(score) = self.scores.remove(member) else {
            return false;
        };
        let i = self.position(score, member).expect("a member is where its score says");
        self.order.remove(i);
        true
    }

    pub fn score(&self, member: &[u8]) -> Option<f64> {
        self.scores.get(member).copied()
    }

    /// The member's rank: how many members come before it.
    pub fn rank(&self, member: &[u8]) -> Option<usize> {
        let score = self.score(member)?;
        self.position(score, member).ok()
    }

    /// The members ranked from `start` to `stop` inclusive, with their scores. Negative
    /// ranks count back from the end, and any part of the range outside the set is left out.
    pub fn range(&self, start: i64, stop: i64) -> &[(f64, Vec<u8>)] {
        let len = self.order.len() as i64;
        let resolve = |rank: i64| if rank < 0 { len + rank } else { rank };
        let (start, stop) = (resolve(start).max(0), resolve(stop).min(len - 1));
        if start > stop {
            return &[];
        }
        &self.order[start as usize..=stop as usize]
    }

    /// Every member with its score, in order.
    pub fn iter(&self) -> impl Iterator<Item = (f64, &[u8])> {
        self.order.iter().map(|(score, member)| (*score, member.as_slice()))
    }
}

/// Parses a score: a decimal number, or `inf`, `+inf` or `-inf`. NaN isn't a score.
pub fn parse_score(arg: &[u8]) -> Option<f64> {
    let text = std::str::from_utf8(arg).ok()?;
    let score: f64 = match text.to_ascii_lowercase().as_str() {
        "inf" | "+inf" => f64::INFINITY,
        "-inf" => f64::NEG_INFINITY,
        // Rust also reads words like "infinity" and "nan", which Redis doesn't.
        lower if lower.contains(|c: char| c.is_ascii_alphabetic() && c != 'e') => return None,
        _ => text.parse().ok()?,
    };
    (!score.is_nan()).then_some(score)
}

/// Formats a score as Redis does: the shortest digits that read back as the same number,
/// written out in full for ordinary magnitudes and with an exponent for very large or
/// small ones, so 30.1 is "30.1" and 1e+30 is "1e+30".
pub fn format_score(score: f64) -> String {
    if score.is_infinite() {
        return if score > 0.0 { "inf" } else { "-inf" }.to_string();
    }
    if score == 0.0 {
        return if score.is_sign_negative() { "-0" } else { "0" }.to_string();
    }
    // `{:e}` gives the shortest digits, as "d.ddde<exponent>".
    let formatted = format!("{:e}", score.abs());
    let (mantissa, exponent) = formatted.split_once('e').unwrap();
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let exponent: i32 = exponent.parse().unwrap();
    let n = digits.len() as i32;
    // The number is `digits` times 10 to the power `k`.
    let k = exponent - (n - 1);
    let sign = if score < 0.0 { "-" } else { "" };
    let distance = (k + n - 1).abs();
    let body = if k >= 0 && distance < n + 7 {
        // A whole number, written out with its trailing zeros.
        format!("{digits}{}", "0".repeat(k as usize))
    } else if k < 0 && (k > -7 || distance < 4) {
        let point = n + k;
        if point <= 0 {
            format!("0.{}{digits}", "0".repeat((-point) as usize))
        } else {
            format!("{}.{}", &digits[..point as usize], &digits[point as usize..])
        }
    } else {
        let max_digits = if score < 0.0 { 17 } else { 18 };
        let digits = &digits[..digits.len().min(max_digits)];
        let (first, rest) = digits.split_at(1);
        let point = if rest.is_empty() { String::new() } else { format!(".{rest}") };
        let exp_sign = if exponent < 0 { '-' } else { '+' };
        format!("{first}{point}e{exp_sign}{}", exponent.abs())
    };
    format!("{sign}{body}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn members(set: &SortedSet) -> Vec<(f64, String)> {
        set.iter()
            .map(|(score, member)| (score, String::from_utf8(member.to_vec()).unwrap()))
            .collect()
    }

    #[test]
    fn keeps_members_in_order_of_score_then_name() {
        let mut set = SortedSet::default();
        assert!(set.insert(b"foo", 100.0));
        assert!(set.insert(b"bar", 100.0));
        assert!(set.insert(b"baz", 20.0));
        assert!(set.insert(b"caz", 30.1));
        assert!(set.insert(b"paz", 40.2));
        let names: Vec<String> = members(&set).into_iter().map(|(_, m)| m).collect();
        assert_eq!(names, ["baz", "caz", "paz", "bar", "foo"]);
        assert_eq!(set.rank(b"baz"), Some(0));
        assert_eq!(set.rank(b"bar"), Some(3));
        assert_eq!(set.rank(b"foo"), Some(4));
        assert_eq!(set.rank(b"missing"), None);
        assert_eq!(set.len(), 5);
    }

    #[test]
    fn updating_a_score_moves_the_member() {
        let mut set = SortedSet::default();
        set.insert(b"a", 1.0);
        set.insert(b"b", 2.0);
        assert!(!set.insert(b"a", 3.0));
        assert_eq!(members(&set), [(2.0, "b".into()), (3.0, "a".into())]);
        assert_eq!(set.score(b"a"), Some(3.0));
        assert_eq!(set.len(), 2);
        // -0 and 0 are the same score, so the name decides.
        set.insert(b"z", -0.0);
        set.insert(b"y", 0.0);
        assert_eq!(set.rank(b"y"), Some(0));
    }

    #[test]
    fn removes_members() {
        let mut set = SortedSet::default();
        set.insert(b"a", 1.0);
        set.insert(b"b", 1.0);
        assert!(set.remove(b"a"));
        assert!(!set.remove(b"a"));
        assert_eq!(members(&set), [(1.0, "b".into())]);
        assert!(set.remove(b"b"));
        assert!(set.is_empty());
    }

    #[test]
    fn ranges_by_rank() {
        let mut set = SortedSet::default();
        for (i, name) in ["a", "b", "c", "d", "e"].iter().enumerate() {
            set.insert(name.as_bytes(), i as f64);
        }
        let names = |start, stop| -> Vec<&[u8]> {
            set.range(start, stop).iter().map(|(_, m)| m.as_slice()).collect()
        };
        assert_eq!(names(0, 2), [b"a", b"b", b"c"]);
        assert_eq!(names(2, 100), [b"c", b"d", b"e"]);
        assert_eq!(names(-2, -1), [b"d", b"e"]);
        assert_eq!(names(0, -3), [b"a", b"b", b"c"]);
        assert_eq!(names(-100, 0), [b"a"]);
        assert!(names(3, 1).is_empty());
        assert!(names(5, 10).is_empty());
        assert!(SortedSet::default().range(0, -1).is_empty());
    }

    #[test]
    fn parses_scores() {
        assert_eq!(parse_score(b"10.5"), Some(10.5));
        assert_eq!(parse_score(b"-3"), Some(-3.0));
        assert_eq!(parse_score(b"1e3"), Some(1000.0));
        assert_eq!(parse_score(b"3663832614298053"), Some(3663832614298053.0));
        assert_eq!(parse_score(b"inf"), Some(f64::INFINITY));
        assert_eq!(parse_score(b"+INF"), Some(f64::INFINITY));
        assert_eq!(parse_score(b"-inf"), Some(f64::NEG_INFINITY));
        for bad in ["nan", "NaN", "abc", "", "1.2.3", "infinity", "10x"] {
            assert_eq!(parse_score(bad.as_bytes()), None, "{bad}");
        }
    }

    #[test]
    fn formats_scores_as_redis_does() {
        for (score, text) in [
            (30.1, "30.1"),
            (24.34, "24.34"),
            (100.99, "100.99"),
            (10.0, "10"),
            (0.0, "0"),
            (-1.5, "-1.5"),
            (3663832614298053.0, "3663832614298053"),
            (0.001, "0.001"),
            (0.000123, "0.000123"),
            (0.0001234, "1.234e-4"),
            (1.5e-8, "1.5e-8"),
            (1.5e-10, "1.5e-10"),
            (1e16, "1e+16"),
            (1e300, "1e+300"),
            (1e30, "1e+30"),
            (123456789.0, "123456789"),
            (f64::INFINITY, "inf"),
            (f64::NEG_INFINITY, "-inf"),
            (2.294471561908722, "2.294471561908722"),
            (-0.08849412202835083, "-0.08849412202835083"),
        ] {
            assert_eq!(format_score(score), text, "{score:?}");
        }
    }
}
