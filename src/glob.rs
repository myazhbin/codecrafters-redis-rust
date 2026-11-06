//! Glob-style patterns, as KEYS and CONFIG GET match names against them: `*` matches any
//! run of characters, `?` any one character, `[abc]` any one of a set (`[^abc]` any one
//! not in it, and `[a-z]` a range), and `\` makes the next character match only itself.

/// Whether all of `string` matches `pattern`, ignoring ASCII case if `nocase` is set.
pub fn matches(pattern: &[u8], string: &[u8], nocase: bool) -> bool {
    let same = |a: u8, b: u8| a == b || (nocase && a.eq_ignore_ascii_case(&b));
    let (mut p, mut s) = (0, 0);
    // After a `*`: where the pattern goes on, and how much of the string the `*` has
    // taken so far. On a mismatch, the `*` takes one more character and matching resumes.
    // Everything else matches exactly one character, so only the last `*` needs retrying.
    let mut star = None;
    while s < string.len() {
        let next = match pattern.get(p) {
            Some(b'*') => {
                // Several stars in a row match the same as one.
                while pattern.get(p) == Some(&b'*') {
                    p += 1;
                }
                star = Some((p, s));
                continue;
            }
            Some(b'?') => Some(p + 1),
            Some(b'[') => match_class(pattern, p + 1, string[s], nocase),
            Some(b'\\') if p + 1 < pattern.len() => same(pattern[p + 1], string[s]).then_some(p + 2),
            Some(&c) => same(c, string[s]).then_some(p + 1),
            None => None,
        };
        match (next, star) {
            (Some(next), _) => {
                p = next;
                s += 1;
            }
            (None, Some((after_star, taken))) => {
                p = after_star;
                s = taken + 1;
                star = Some((after_star, taken + 1));
            }
            (None, None) => return false,
        }
    }
    // The string is used up, so the rest of the pattern has to match nothing.
    pattern[p..].iter().all(|&c| c == b'*')
}

/// Matches `c` against the character class starting at `pattern[start]`, just after its
/// `[`. Returns where the pattern goes on after the class's `]` if `c` is in the class.
/// As in Redis, a class with no `]` runs to the end of the pattern.
fn match_class(pattern: &[u8], start: usize, c: u8, nocase: bool) -> Option<usize> {
    let fold = |b: u8| if nocase { b.to_ascii_lowercase() } else { b };
    let negated = pattern.get(start) == Some(&b'^');
    let mut i = start + usize::from(negated);
    let mut found = false;
    while i < pattern.len() && pattern[i] != b']' {
        if pattern[i] == b'\\' && i + 1 < pattern.len() {
            found |= pattern[i + 1] == c;
            i += 2;
        } else if i + 2 < pattern.len() && pattern[i + 1] == b'-' {
            let (a, b) = (fold(pattern[i]), fold(pattern[i + 2]));
            found |= (a.min(b)..=a.max(b)).contains(&fold(c));
            i += 3;
        } else {
            found |= fold(pattern[i]) == fold(c);
            i += 1;
        }
    }
    // Skip the `]`, if there is one.
    let after = (i + 1).min(pattern.len());
    (found != negated).then_some(after)
}

#[cfg(test)]
mod tests {
    use super::matches;

    fn m(pattern: &str, string: &str) -> bool {
        matches(pattern.as_bytes(), string.as_bytes(), false)
    }

    #[test]
    fn star_matches_any_run() {
        assert!(m("*", ""));
        assert!(m("*", "foo"));
        assert!(m("f*", "foo"));
        assert!(m("f*", "f"));
        assert!(!m("f*", "bar"));
        assert!(m("*o", "foo"));
        assert!(m("f*o*r", "foobar"));
        assert!(m("a**b", "axxb"));
        assert!(!m("a*b", "axxc"));
        assert!(m("*ab*ab", "abxabab"));
    }

    #[test]
    fn literals_and_question_marks() {
        assert!(m("foo", "foo"));
        assert!(!m("foo", "fo"));
        assert!(!m("foo", "fooo"));
        assert!(m("h?llo", "hello"));
        assert!(!m("h?llo", "hllo"));
        assert!(!m("", "a"));
        assert!(m("", ""));
    }

    #[test]
    fn character_classes() {
        assert!(m("h[ae]llo", "hello"));
        assert!(m("h[ae]llo", "hallo"));
        assert!(!m("h[ae]llo", "hillo"));
        assert!(m("h[^e]llo", "hallo"));
        assert!(!m("h[^e]llo", "hello"));
        assert!(m("h[a-c]llo", "hbllo"));
        assert!(m("h[c-a]llo", "hbllo"));
        assert!(!m("h[a-c]llo", "hdllo"));
        assert!(m("[\\]]", "]"));
        assert!(m("x[ab", "xb"));
    }

    #[test]
    fn escapes() {
        assert!(m("a\\*b", "a*b"));
        assert!(!m("a\\*b", "axb"));
        assert!(m("a\\?", "a?"));
        assert!(!m("a\\?", "ab"));
    }

    #[test]
    fn case() {
        assert!(!m("DIR", "dir"));
        assert!(matches(b"DIR", b"dir", true));
        assert!(matches(b"D[H-J]r", b"dir", true));
    }
}
