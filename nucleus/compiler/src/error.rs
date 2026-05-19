//! Shared parser error types.
//!
//! Factored out of `algo/parser.rs` (TASK-0007) so the schedule
//! sublanguage parser (TASK-0008) can reuse the same type instead of
//! defining a near-clone. The shape — `(line, column, kind, message)`
//! — is identical between the two parsers; the kinds are deliberately
//! sublanguage-agnostic.
//!
//! Both `crate::algo` and `crate::sched` re-export [`ParseError`] and
//! [`ParseErrorKind`] from their own surfaces so callers don't have to
//! reach into `crate::error` directly. The aliasing is a stylistic
//! choice; the underlying type is the same.

/// A parse error with `(line, column)` source location.
///
/// Only the first error in the source is reported; multi-error
/// reporting is a follow-up across both parsers. The `kind`
/// distinguishes the broad failure category so tests can match on a
/// variant without scraping a message string.
#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub line: usize,
    pub column: usize,
    pub kind: ParseErrorKind,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseErrorKind {
    /// Unexpected token / character. The default for combinator
    /// failures that don't map to a more specific variant.
    Unexpected,
    /// Unexpected end of input mid-construct.
    UnexpectedEof,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "parse error at line {}, column {}: {}",
            self.line, self.column, self.message
        )
    }
}

impl std::error::Error for ParseError {}

/// 1-based `(line, column)` for a byte offset into `src`. ASCII source
/// per both grammars' lexical rules; counting bytes works for ASCII
/// columns.
pub fn offset_to_line_col(src: &str, offset: usize) -> (usize, usize) {
    let mut line = 1usize;
    let mut col = 1usize;
    for (i, ch) in src.char_indices() {
        if i >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// Map a chumsky `Simple<char>` error list to a single [`ParseError`].
///
/// Both parsers want the same shape of mapping: take the first error,
/// resolve its span to `(line, column)`, classify as
/// `Unexpected`/`UnexpectedEof`, and keep chumsky's default Display
/// for the message. Lives here to avoid duplicating the helper.
pub fn map_first_chumsky_error(src: &str, errors: Vec<chumsky::error::Simple<char>>) -> ParseError {
    let err = errors
        .into_iter()
        .next()
        .expect("chumsky returns a non-empty error list on failure");
    let span = err.span();
    let offset = span.start.min(src.len());
    let (line, column) = offset_to_line_col(src, offset);

    let kind = match err.reason() {
        chumsky::error::SimpleReason::Unexpected if err.found().is_none() => {
            ParseErrorKind::UnexpectedEof
        }
        _ => ParseErrorKind::Unexpected,
    };

    let message = err.to_string();

    ParseError {
        line,
        column,
        kind,
        message,
    }
}

// --------------------------------------------------------------------
// Fuzzy-match "did you mean?" suggestions (TASK-0096)
// --------------------------------------------------------------------
//
// Shared, zero-dependency edit-distance helper for unknown-name
// diagnostics. Lives in `error` (not `link`) deliberately: the sibling
// SchedLowerError fuzzy-match (TASK-0198) reuses `suggest` verbatim
// against `sched/lower.rs`'s in-hand schedule symbol tables — one
// implementation, no duplication. Per decision-0001 (zero-runtime-dep
// ethos) this is a tiny in-house function, NOT a new crate
// (`strsim`/`levenshtein`).

/// Levenshtein edit distance between two strings, compared by
/// `char` (the algorithm/schedule grammars are ASCII, but char-wise
/// is correct for any input and costs nothing extra here).
///
/// Standard O(n·m) dynamic program with a single rolling row, so
/// memory is O(min-ish) and the result is a deterministic pure
/// function of the inputs — no allocation-order or hash-iteration
/// dependence. Plain Levenshtein (insertion / deletion /
/// substitution), NOT Damerau: a transposition costs 2 here. That is
/// deliberate — the candidate sets this serves are short identifier
/// tables where a transposition is a rare typo class relative to the
/// three primitive edits, and keeping the helper to the minimal DP is
/// the decision-0001 discipline. If a future task shows transpositions
/// dominate, widen then, not speculatively.
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() {
        return b.len();
    }
    if b.is_empty() {
        return a.len();
    }

    // `prev[j]` = edit distance between a[..i] and b[..j].
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr: Vec<usize> = vec![0; b.len() + 1];

    for (i, &ca) in a.iter().enumerate() {
        curr[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            curr[j + 1] = (prev[j + 1] + 1) // deletion
                .min(curr[j] + 1) // insertion
                .min(prev[j] + cost); // substitution / match
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
}

/// Pick the single closest candidate to `name` as a "did you mean?"
/// suggestion, or `None` if nothing is close enough.
///
/// # Determinism (reproducibility gate — PRD §10.1)
///
/// The candidate iterator is collected and **sorted lexicographically**
/// before selection, so the result is a deterministic pure function of
/// `(name, the candidate multiset)` regardless of the caller's
/// iteration order. The closest candidate by [`levenshtein`] distance
/// wins; among equal-distance candidates the **lexicographically-first**
/// is chosen (first survivor of the sorted scan with a strict `<`
/// comparison). There is NO `HashMap`/`HashSet` in the selection path,
/// so no hash-iteration order can leak into the chosen suggestion.
///
/// # Threshold
///
/// A candidate is only offered if its distance is within
/// `max(1, name.chars().count() / 3)` — i.e. roughly "a typo or two,
/// scaled to the name's length". This catches single/double-char typos
/// (`food`→`foo`, `kernl`→`kernel`) while refusing to suggest an
/// unrelated symbol for a wholly-different name. The constant `1` floor
/// keeps short names (≤ 3 chars) correctable by exactly one edit.
pub fn suggest<'a, I>(name: &str, candidates: I) -> Option<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut sorted: Vec<&str> = candidates.into_iter().collect();
    sorted.sort_unstable();

    let bound = (name.chars().count() / 3).max(1);

    let mut best: Option<(usize, &str)> = None;
    for cand in sorted {
        let d = levenshtein(name, cand);
        match best {
            // Strict `<` only: the first (lexicographically-smallest,
            // because `sorted`) candidate at the minimal distance is
            // kept — deterministic tie-break.
            Some((bd, _)) if d < bd => best = Some((d, cand)),
            None => best = Some((d, cand)),
            _ => {}
        }
    }

    match best {
        Some((d, cand)) if d <= bound => Some(cand.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod fuzzy_tests {
    use super::{levenshtein, suggest};

    #[test]
    fn levenshtein_known_pairs() {
        // Distance 0: identical.
        assert_eq!(levenshtein("foo", "foo"), 0);
        // Empty cases.
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("", "abc"), 3);
        assert_eq!(levenshtein("abc", ""), 3);
        // Single substitution.
        assert_eq!(levenshtein("foo", "fou"), 1);
        // Single insertion / deletion (symmetric).
        assert_eq!(levenshtein("foo", "fooo"), 1);
        assert_eq!(levenshtein("fooo", "foo"), 1);
        // Transposition costs 2 under plain Levenshtein (NOT Damerau)
        // — this pins the documented design choice.
        assert_eq!(levenshtein("ab", "ba"), 2);
        // Classic textbook value.
        assert_eq!(levenshtein("kitten", "sitting"), 3);
    }

    #[test]
    fn levenshtein_is_symmetric_and_unicode_safe() {
        assert_eq!(levenshtein("abcd", "abdc"), levenshtein("abdc", "abcd"));
        // char-wise, not byte-wise: a multibyte char is one edit.
        assert_eq!(levenshtein("café", "cafe"), 1);
    }

    #[test]
    fn suggest_typo_returns_closest() {
        // One insertion, within bound max(1, 4/3)=1.
        assert_eq!(
            suggest("fooo", ["foo", "barbaz"]),
            Some("foo".to_string())
        );
    }

    #[test]
    fn suggest_unrelated_returns_none() {
        // `xyz` vs `foo`/`bar`: distance 3, bound max(1,3/3)=1 → None.
        assert_eq!(suggest("xyz", ["foo", "bar"]), None);
    }

    #[test]
    fn suggest_empty_candidates_returns_none() {
        let empty: [&str; 0] = [];
        assert_eq!(suggest("foo", empty), None);
    }

    #[test]
    fn suggest_tie_break_is_lexicographically_first() {
        // `ac` is distance 1 from BOTH `ab` and `ad` (one
        // substitution each) and from `aa`. Among the equal-distance
        // set the lexicographically-first must win, deterministically,
        // regardless of input order.
        assert_eq!(
            suggest("ac", ["ad", "ab", "aa"]),
            Some("aa".to_string())
        );
        assert_eq!(
            suggest("ac", ["aa", "ab", "ad"]),
            Some("aa".to_string())
        );
        // Same multiset, reversed: identical result (determinism).
        assert_eq!(
            suggest("ac", ["ad", "ab", "aa"]),
            suggest("ac", ["aa", "ad", "ab"])
        );
    }

    #[test]
    fn suggest_bound_scales_with_name_length() {
        // Long name tolerates 2 edits: len 9, bound = 9/3 = 3.
        assert_eq!(
            suggest("activatns", ["activations"]),
            Some("activations".to_string())
        );
        // Short name only tolerates the floor of 1 edit.
        assert_eq!(suggest("ab", ["abcd"]), None);
    }
}
