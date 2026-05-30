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
//!
//! Both parsers return the multi-error owner [`ParseErrors`] (a
//! non-empty, deterministically-ordered `Vec<ParseError>`): the
//! algorithm parser recovers at statement/item boundaries
//! (TASK-0080 / TASK-0081) and the schedule parser at the directive
//! `;` boundary (TASK-0087), each reporting every error in one pass.
//! The atomic per-error mapping (`map_one_chumsky_error`) and the
//! deterministic message builder (`chumsky_message`) are shared by
//! both via [`map_all_chumsky_errors`].

/// A parse error with `(line, column)` source location.
///
/// One element of a parse failure. Both the algorithm parser
/// (TASK-0080 / TASK-0081) and the schedule parser (TASK-0087)
/// recover at their respective `;` boundaries and return *all* errors
/// found in one pass, bundled in [`ParseErrors`]. The `kind`
/// distinguishes the broad failure category so tests can match on a
/// variant without scraping a message string.
#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub line: usize,
    pub column: usize,
    pub kind: ParseErrorKind,
    pub message: String,
}

/// A non-empty, ordered bundle of [`ParseError`]s from a single parse
/// pass.
///
/// Returned by both `parse_algo` and `parse_sched` so that one
/// syntactic failure does not hide the rest of the program's errors
/// (multi-error reporting + error recovery — in chumsky these are one
/// coherent change; TASK-0080 / TASK-0081 for the algorithm parser,
/// TASK-0087 for the schedule parser). The `Ok` type of each parser
/// is *unchanged* (`AlgoAst` / `SchedAst`); only the `Err` type became
/// this owner, so every success-path caller compiles untouched and
/// only the parsers' negative tests migrate.
///
/// # Invariants
///
/// - **Non-empty**: a `ParseErrors` is only constructed on a parse
///   failure, which always yields at least one error.
/// - **Deterministic order**: errors are kept in chumsky's positional
///   order (earliest source offset first), then deduplicated by full
///   value while preserving first-seen order. No `HashMap`/`HashSet`
///   touches the error path, so the error set *and its order* are a
///   pure function of the source (reproducibility gate, PRD §10.1).
///
/// `Deref`s to `[ParseError]` so callers can iterate, index, or call
/// slice methods directly; [`ParseErrors::first`] returns the
/// earliest error for the common "I just want the primary error"
/// case (this is what the migrated single-error negative tests use —
/// same per-error discriminating power as before).
#[derive(Debug, Clone, PartialEq)]
pub struct ParseErrors(pub Vec<ParseError>);

impl ParseErrors {
    /// The first (earliest, lowest source offset) error. Total because
    /// the type is only ever constructed non-empty.
    pub fn first(&self) -> &ParseError {
        self.0
            .first()
            .expect("ParseErrors is constructed non-empty (invariant)")
    }

    /// All errors, in deterministic positional order.
    pub fn errors(&self) -> &[ParseError] {
        &self.0
    }
}

impl std::ops::Deref for ParseErrors {
    type Target = [ParseError];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::fmt::Display for ParseErrors {
    /// One `ParseError` per line so a multi-error failure is readable
    /// when surfaced as a single formatted string (the driver
    /// additionally prefixes each line via its own iteration; this
    /// Display is the fallback for any caller that just `{}`s the
    /// whole bundle).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, e) in self.0.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{e}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ParseErrors {}

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

/// Build a **deterministic** human message for a chumsky
/// `Simple<char>`.
///
/// # Why not `Simple::to_string()`
///
/// chumsky 0.9's `impl Display for Simple` iterates `self.expected`,
/// which is an internal `HashSet`, so its "expected one of …" list
/// comes out in *hash-iteration order* — i.e. it differs between
/// otherwise-identical parses of the same source. That is a direct
/// reproducibility-gate violation (PRD §10.1): two builds of the same
/// program would emit byte-different diagnostics. We therefore do NOT
/// use chumsky's `Display`; we reconstruct the message from the
/// structured accessors with the expected set **sorted**, so the
/// string is a pure deterministic function of the error.
///
/// (Both parsers share this helper via [`map_all_chumsky_errors`], so
/// this is the correct root-cause determinism fix, not a parser-local
/// workaround.)
fn chumsky_message(err: &chumsky::error::Simple<char>) -> String {
    use chumsky::error::SimpleReason;

    // For a custom error (e.g. our `ident()` keyword rejection,
    // `int_lit` overflow) chumsky carries the message verbatim in
    // `SimpleReason::Custom`; that is already deterministic and more
    // specific than the generic expected-set rendering.
    if let SimpleReason::Custom(msg) = err.reason() {
        return msg.clone();
    }
    if let SimpleReason::Unclosed { delimiter, .. } = err.reason() {
        return format!("unclosed delimiter `{delimiter}`");
    }

    let found = match err.found() {
        Some(c) => format!("found {:?}", c.to_string()),
        None => "found end of input".to_string(),
    };

    // Sort the rendered expected strings for a deterministic message
    // order (chumsky 0.9's `Simple::expected()` is HashSet-backed, so
    // its native iteration order is non-deterministic). The sort key is
    // the rendered string itself; `None` renders as "end of input" and
    // takes whatever lexicographic position that string has — its exact
    // position is irrelevant, only that the order is total + stable.
    let mut expected: Vec<String> = err
        .expected()
        .map(|opt| match opt {
            Some(c) => format!("{:?}", c.to_string()),
            None => "end of input".to_string(),
        })
        .collect();
    expected.sort();
    expected.dedup();

    match expected.len() {
        0 => found,
        1 => format!("{found} but expected {}", expected[0]),
        _ => format!("{found} but expected one of {}", expected.join(", ")),
    }
}

/// Map a single chumsky `Simple<char>` to a [`ParseError`].
///
/// The atomic per-error mapping used by [`map_all_chumsky_errors`]
/// (both parsers' multi-error surface): resolve the error's span to
/// `(line, column)`, classify as `Unexpected`/`UnexpectedEof`, and
/// build a **deterministic** message (see [`chumsky_message`]). Pure
/// function of `(src, err)` — no allocation-order or hash dependence.
fn map_one_chumsky_error(src: &str, err: &chumsky::error::Simple<char>) -> ParseError {
    let span = err.span();
    let offset = span.start.min(src.len());
    let (line, column) = offset_to_line_col(src, offset);

    let kind = match err.reason() {
        chumsky::error::SimpleReason::Unexpected if err.found().is_none() => {
            ParseErrorKind::UnexpectedEof
        }
        _ => ParseErrorKind::Unexpected,
    };

    let message = chumsky_message(err);

    ParseError {
        line,
        column,
        kind,
        message,
    }
}

/// Map *every* chumsky `Simple<char>` to a [`ParseError`], bundled in
/// a non-empty, deterministically-ordered [`ParseErrors`].
///
/// Used by both parsers after `parse_recovery` (algorithm:
/// TASK-0080 / TASK-0081; schedule: TASK-0087). chumsky already
/// yields errors in positional order
/// (earliest source offset first); we preserve that and then
/// **deduplicate by full value while keeping first-seen order**.
///
/// # Why dedup, and why this dedup
///
/// chumsky can emit several `Simple<char>` at the *same* offset (one
/// per failed alternative in a `choice`), and recovery can re-surface
/// an error if a retry fails at the same spot. A naive pass would
/// spam the user with near-duplicate lines at one position. We
/// collapse exact `(line, column, kind, message)` duplicates. The
/// dedup is an order-preserving linear scan over a `Vec` (no
/// `HashSet`), so the surviving set *and its order* are a pure
/// deterministic function of the source (reproducibility gate). We
/// dedup on the *mapped* `ParseError` (post-Display) rather than the
/// raw `Simple` because two different `Simple`s can render to the
/// identical user-facing line; collapsing those is the user-visible
/// invariant we want to pin.
///
/// # Panics
///
/// Panics if `errors` is empty — chumsky guarantees a non-empty list
/// when parsing fails, and this is only called on the failure path
/// (the `ParseErrors` non-empty invariant). This is an
/// earlier-pass-guaranteed invariant, not diagnosable user input
/// (decision-0003).
pub fn map_all_chumsky_errors(src: &str, errors: Vec<chumsky::error::Simple<char>>) -> ParseErrors {
    assert!(
        !errors.is_empty(),
        "chumsky returns a non-empty error list on parse failure"
    );
    let mut out: Vec<ParseError> = Vec::with_capacity(errors.len());
    for err in &errors {
        let mapped = map_one_chumsky_error(src, err);
        // Order-preserving exact-duplicate suppression. O(n^2) on the
        // error count, which is tiny (a handful of syntax errors per
        // program); a `HashSet` would be faster but would risk
        // leaking iteration order into the surfaced set — the
        // determinism contract forbids that. Linear scan it is.
        if !out.contains(&mapped) {
            out.push(mapped);
        }
    }
    ParseErrors(out)
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
        assert_eq!(suggest("fooo", ["foo", "barbaz"]), Some("foo".to_string()));
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
        assert_eq!(suggest("ac", ["ad", "ab", "aa"]), Some("aa".to_string()));
        assert_eq!(suggest("ac", ["aa", "ab", "ad"]), Some("aa".to_string()));
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
