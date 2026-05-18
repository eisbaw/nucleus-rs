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
pub fn map_first_chumsky_error(
    src: &str,
    errors: Vec<chumsky::error::Simple<char>>,
) -> ParseError {
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
