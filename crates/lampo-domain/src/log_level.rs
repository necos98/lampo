//! Severity of a log line, guessed from its text, and the filter that keeps lines by severity
//! (`lampo logs --level`). The rules are those of the desktop viewer
//! (`apps/desktop/src/lib/logs/level.ts`): change both together, the tests share the same cases.

/// Keywords that make a line an error (whole words, any case).
const ERROR_WORDS: [&str; 5] = ["error", "fatal", "critical", "emerg", "alert"];
/// Keywords that make a line a warning (whole words, any case).
const WARN_WORDS: [&str; 4] = ["warn", "warning", "deprecated", "notice"];
/// First digit of the HTTP statuses that count as errors in an access log (5xx).
const SERVER_ERROR_DIGIT: char = '5';

/// Severity of a highlighted line; ordinary lines have none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum LogLevel {
    Warn,
    Error,
}

impl LogLevel {
    /// An error keyword or an HTTP 5xx status between whitespace wins over a warning keyword.
    pub fn of(text: &str) -> Option<Self> {
        if has_word(text, &ERROR_WORDS) || has_server_error_status(text) {
            Some(Self::Error)
        } else if has_word(text, &WARN_WORDS) {
            Some(Self::Warn)
        } else {
            None
        }
    }
}

/// Which lines are kept by severity: every line, warnings and errors, or errors only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum LevelFilter {
    #[default]
    All,
    Warnings,
    Errors,
}

impl LevelFilter {
    /// The lowest severity kept (`None`: every line, highlighted or not).
    fn min_level(self) -> Option<LogLevel> {
        match self {
            Self::All => None,
            Self::Warnings => Some(LogLevel::Warn),
            Self::Errors => Some(LogLevel::Error),
        }
    }

    /// Whether a line of `level` is kept.
    pub fn passes(self, level: Option<LogLevel>) -> bool {
        level >= self.min_level()
    }

    /// Whether the line `text` is kept.
    pub fn keeps(self, text: &str) -> bool {
        self == Self::All || self.passes(LogLevel::of(text))
    }
}

/// A word character of a JavaScript `\b` without the `u` flag: ASCII letters, digits, `_`.
fn is_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Whether one of `words` (lowercase ASCII) is a whole word of `text`, ignoring ASCII case.
fn has_word(text: &str, words: &[&str]) -> bool {
    text.split(|c: char| !is_word_char(c)).any(|word| words.iter().any(|w| word.eq_ignore_ascii_case(w)))
}

/// A JavaScript `\s`: the ECMAScript white space and line terminators (not `char::is_whitespace`,
/// which also counts U+0085 and not U+FEFF).
fn is_js_space(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\u{B}' | '\u{C}' | '\r' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'..='\u{200A}' | '\u{2028}' | '\u{2029}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}')
}

/// A `5xx` status between whitespace, as access logs write it (`"GET / HTTP/1.1" 500 1021`).
fn has_server_error_status(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    chars.windows(5).any(|w| is_js_space(w[0]) && w[1] == SERVER_ERROR_DIGIT && w[2].is_ascii_digit() && w[3].is_ascii_digit() && is_js_space(w[4]))
}

#[cfg(test)]
mod tests {
    use super::*;

    // The cases of `apps/desktop/src/lib/logs/level.test.ts`.

    #[test]
    fn error_keywords_and_5xx_statuses_are_errors() {
        assert_eq!(LogLevel::of("[php:error] PHP Fatal error: Uncaught Error"), Some(LogLevel::Error));
        assert_eq!(LogLevel::of("[core:CRITICAL] boom"), Some(LogLevel::Error));
        assert_eq!(LogLevel::of("127.0.0.1 - - \"GET /checkout HTTP/1.1\" 500 1021"), Some(LogLevel::Error));
    }

    #[test]
    fn warning_keywords_are_warnings() {
        assert_eq!(LogLevel::of("PHP Warning: Undefined variable $user"), Some(LogLevel::Warn));
        assert_eq!(LogLevel::of("PHP Deprecated: Return type of Foo::count()"), Some(LogLevel::Warn));
        assert_eq!(LogLevel::of("[core:notice] AH00094: Command line"), Some(LogLevel::Warn));
    }

    #[test]
    fn errors_win_over_warnings_on_the_same_line() {
        assert_eq!(LogLevel::of("warning: fatal error ahead"), Some(LogLevel::Error));
    }

    #[test]
    fn ordinary_empty_and_look_alike_lines_are_not_highlighted() {
        for text in ["127.0.0.1 - - \"GET / HTTP/1.1\" 200 5123", "", "errors_total=0 terrorism warned", "port 5000 ready", "città è pronta ✓"] {
            assert_eq!(LogLevel::of(text), None, "{text}");
        }
    }

    #[test]
    fn the_level_filter_keeps_lines_at_or_above_its_severity() {
        let levels = [None, Some(LogLevel::Warn), Some(LogLevel::Error)];
        let kept = |filter: LevelFilter| levels.map(|l| filter.passes(l));
        assert_eq!(kept(LevelFilter::All), [true, true, true]);
        assert_eq!(kept(LevelFilter::Warnings), [false, true, true]);
        assert_eq!(kept(LevelFilter::Errors), [false, false, true]);
    }

    #[test]
    fn keywords_next_to_non_ascii_letters_and_status_at_the_edges() {
        // Like a JavaScript `\b`, an accented letter is not part of a word.
        assert_eq!(LogLevel::of("erroreè"), None);
        assert_eq!(LogLevel::of("àerror"), Some(LogLevel::Error));
        // The status needs whitespace on both sides, a no-break space included.
        assert_eq!(LogLevel::of("500 at the start"), None);
        assert_eq!(LogLevel::of("ends with 503"), None);
        assert_eq!(LogLevel::of("status\u{A0}502\u{A0}bad gateway"), Some(LogLevel::Error));
        // JavaScript's `\s`: U+FEFF is white space, U+0085 is not.
        assert_eq!(LogLevel::of("x\u{FEFF}500\u{FEFF}y"), Some(LogLevel::Error));
        assert_eq!(LogLevel::of("x\u{85}500\u{85}y"), None);
        assert!(LevelFilter::Warnings.keeps("Notice: x") && !LevelFilter::Errors.keeps("Notice: x") && LevelFilter::All.keeps(""));
    }
}
