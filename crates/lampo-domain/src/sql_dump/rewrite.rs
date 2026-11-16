//! Streaming rewrite of a dump made by another server, applied on its way into `mysql` (the file
//! on disk is never changed).
//!
//! The rewriter works on lines and only ever looks at the start of a line (the first
//! [`LINE_HEAD_BYTES`]): the rest of a line passes through untouched, so memory stays constant
//! whatever the size of an `INSERT` line. mysqldump writes the clauses it rewrites at the start of
//! a statement, and escapes the line breaks of string values (`\n`), so data rows always start
//! with `INSERT` and are never rewritten.

/// Bytes at the start of a line that the rewrite rules may look at.
pub const LINE_HEAD_BYTES: usize = 8 * 1024;

/// First line of the dumps of recent MariaDB releases: the `mysql` client of MySQL rejects it.
pub const MARIADB_SANDBOX_LINE: &[u8] = b"/*M!999999\\- enable the sandbox mode */";

/// Which rewrites to apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DumpRewrite {
    /// Drop MariaDB's sandbox-mode comment from the first line ([`MARIADB_SANDBOX_LINE`]).
    pub drop_sandbox_line: bool,
    /// Remove the `DEFINER=<account>` clause of views, triggers, routines and events, so the
    /// objects belong to the importing user instead of an account that may not exist. Only the
    /// clause in a leading `/*!NNNNN DEFINER=… */` conditional comment (after an optional
    /// `/*!NNNNN CREATE*/`) or in a `CREATE [OR REPLACE] [ALGORITHM=…] DEFINER=…` header is removed.
    pub strip_definers: bool,
}

impl DumpRewrite {
    pub fn is_noop(self) -> bool {
        !self.drop_sandbox_line && !self.strip_definers
    }
}

/// Where the rewriter is within the current line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Position {
    /// Collecting the head of a line (up to [`LINE_HEAD_BYTES`]).
    Head,
    /// Past the head: copying the rest of the line as it is.
    Tail,
}

/// Applies a [`DumpRewrite`] to a dump fed in chunks of any size (split anywhere, even inside a
/// UTF-8 character). Memory is bounded by [`LINE_HEAD_BYTES`].
#[derive(Debug)]
pub struct DumpRewriter {
    rules: DumpRewrite,
    head: Vec<u8>,
    position: Position,
    first_line: bool,
}

impl DumpRewriter {
    pub fn new(rules: DumpRewrite) -> Self {
        Self { rules, head: Vec::with_capacity(LINE_HEAD_BYTES), position: Position::Head, first_line: true }
    }

    /// Rewrites `input` (the next chunk of the dump) into `out`. Bytes of an unfinished line head
    /// are held back until the line ends, the head is full or [`Self::finish`] is called.
    pub fn push(&mut self, mut input: &[u8], out: &mut Vec<u8>) {
        while !input.is_empty() {
            let newline = input.iter().position(|&b| b == b'\n');
            match self.position {
                Position::Tail => {
                    let end = newline.map_or(input.len(), |i| i + 1);
                    out.extend_from_slice(&input[..end]);
                    if newline.is_some() {
                        self.position = Position::Head;
                    }
                    input = &input[end..];
                }
                Position::Head => {
                    let room = LINE_HEAD_BYTES - self.head.len();
                    let end = newline.map_or(input.len(), |i| i + 1).min(room);
                    self.head.extend_from_slice(&input[..end]);
                    input = &input[end..];
                    if self.head.ends_with(b"\n") {
                        self.emit_head(out);
                    } else if self.head.len() == LINE_HEAD_BYTES {
                        self.emit_head(out);
                        self.position = Position::Tail;
                    }
                }
            }
        }
    }

    /// Flushes a last line without a line break.
    pub fn finish(&mut self, out: &mut Vec<u8>) {
        if !self.head.is_empty() {
            self.emit_head(out);
        }
    }

    fn emit_head(&mut self, out: &mut Vec<u8>) {
        let first_line = std::mem::take(&mut self.first_line);
        let dropped = first_line && self.rules.drop_sandbox_line && drop_sandbox_comment(&mut self.head);
        if !dropped {
            if self.rules.strip_definers
                && let Some(clause) = definer_clause(&self.head)
            {
                self.head.drain(clause);
            }
            out.extend_from_slice(&self.head);
        }
        self.head.clear();
    }
}

/// Removes MariaDB's sandbox comment from the start of `line`; `true` when nothing but white
/// space is left (the whole line goes).
fn drop_sandbox_comment(line: &mut Vec<u8>) -> bool {
    if !line.starts_with(MARIADB_SANDBOX_LINE) {
        return false;
    }
    let rest = skip_blanks(line, MARIADB_SANDBOX_LINE.len());
    line.drain(..rest);
    line.iter().all(u8::is_ascii_whitespace)
}

/// The byte range of the `DEFINER=<account>` clause of a statement head (with the blanks after
/// it), if the line starts one.
fn definer_clause(line: &[u8]) -> Option<std::ops::Range<usize>> {
    let start = skip_blanks(line, 0);
    if line[start..].starts_with(b"/*!") { conditional_definer(line, start) } else { create_header_definer(line, start) }
}

/// `/*!50013 DEFINER=… SQL SECURITY DEFINER */`, or `/*!50003 CREATE*/ /*!50017 DEFINER=…*/` (the
/// form of triggers, events and old routine dumps).
fn conditional_definer(line: &[u8], mut at: usize) -> Option<std::ops::Range<usize>> {
    loop {
        let body = skip_blanks(line, skip_digits(line, at + b"/*!".len()));
        if let Some(word_end) = keyword_at(line, body, b"DEFINER") {
            return account_clause(line, body, word_end);
        }
        let after_create = skip_blanks(line, keyword_at(line, body, b"CREATE")?);
        if !line[after_create..].starts_with(b"*/") {
            return None;
        }
        at = skip_blanks(line, after_create + b"*/".len());
        if !line[at..].starts_with(b"/*!") {
            return None;
        }
    }
}

/// `CREATE [OR REPLACE] [ALGORITHM = …] DEFINER=… VIEW|PROCEDURE|FUNCTION|TRIGGER|EVENT …`.
fn create_header_definer(line: &[u8], start: usize) -> Option<std::ops::Range<usize>> {
    let mut at = keyword_at(line, start, b"CREATE")?;
    loop {
        let word = skip_blanks(line, at);
        if word == at {
            return None;
        }
        if let Some(word_end) = keyword_at(line, word, b"DEFINER") {
            return account_clause(line, word, word_end);
        }
        at = if let Some(end) = keyword_at(line, word, b"OR").or_else(|| keyword_at(line, word, b"REPLACE")) {
            end
        } else {
            let after_equals = expect_equals(line, keyword_at(line, word, b"ALGORITHM")?)?;
            bare_word(line, after_equals)?
        };
    }
}

/// `DEFINER = <account>` from `start` (where `DEFINER` begins; `word_end` is past it) through the
/// blanks after the account.
fn account_clause(line: &[u8], start: usize, word_end: usize) -> Option<std::ops::Range<usize>> {
    let account_start = expect_equals(line, word_end)?;
    let end = account(line, account_start)?;
    // The account must end the clause: `` `a`@`b`x `` is not an account this rewrite understands.
    let clause_ends = line.get(end).is_none_or(|b| b.is_ascii_whitespace() || *b == b'*');
    clause_ends.then(|| start..skip_blanks(line, end))
}

/// `user@host` (each part quoted with backticks, single or double quotes, or bare), or
/// `CURRENT_USER` / `CURRENT_USER()`. Returns the position past it.
fn account(line: &[u8], at: usize) -> Option<usize> {
    if let Some(end) = keyword_at(line, at, b"CURRENT_USER") {
        return Some(if line[end..].starts_with(b"()") { end + 2 } else { end });
    }
    let user_end = account_part(line, at)?;
    if line.get(user_end) == Some(&b'@') { account_part(line, user_end + 1) } else { Some(user_end) }
}

fn account_part(line: &[u8], at: usize) -> Option<usize> {
    match line.get(at)? {
        &quote @ (b'`' | b'\'' | b'"') => quoted_end(line, at, quote),
        _ => {
            let end = at + line[at..].iter().take_while(|&&b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'$' | b'.' | b'%' | b'-')).count();
            (end > at).then_some(end)
        }
    }
}

/// Past the closing quote of the quoted text at `at`; a doubled quote is part of the text, as is
/// a backslash-escaped one inside single or double quotes. `None` if it does not close in `line`.
fn quoted_end(line: &[u8], at: usize, quote: u8) -> Option<usize> {
    let mut i = at + 1;
    while i < line.len() {
        match line[i] {
            b'\\' if quote != b'`' => i += 2,
            b if b == quote && line.get(i + 1) == Some(&quote) => i += 2,
            b if b == quote => return Some(i + 1),
            b'\n' => return None,
            _ => i += 1,
        }
    }
    None
}

/// Past `= ` after optional blanks, or `None` if no `=` follows.
fn expect_equals(line: &[u8], at: usize) -> Option<usize> {
    let equals = skip_blanks(line, at);
    (line.get(equals) == Some(&b'=')).then(|| skip_blanks(line, equals + 1))
}

/// Past the identifier-like word at `at` (`UNDEFINED`, `MERGE`, `TEMPTABLE`).
fn bare_word(line: &[u8], at: usize) -> Option<usize> {
    let end = at + line[at..].iter().take_while(|b| b.is_ascii_alphanumeric() || **b == b'_').count();
    (end > at).then_some(end)
}

/// Past `keyword` when `line` has it at `at` (case-insensitive) as a whole word.
fn keyword_at(line: &[u8], at: usize, keyword: &[u8]) -> Option<usize> {
    let end = at + keyword.len();
    let candidate = line.get(at..end)?;
    let whole_word = line.get(end).is_none_or(|b| !(b.is_ascii_alphanumeric() || *b == b'_'));
    (candidate.eq_ignore_ascii_case(keyword) && whole_word).then_some(end)
}

/// Past the spaces and tabs from `at` (a line break ends the head, so it is not skipped).
fn skip_blanks(line: &[u8], at: usize) -> usize {
    at + line.get(at..).unwrap_or_default().iter().take_while(|b| matches!(b, b' ' | b'\t')).count()
}

fn skip_digits(line: &[u8], at: usize) -> usize {
    at + line.get(at..).unwrap_or_default().iter().take_while(|b| b.is_ascii_digit()).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: DumpRewrite = DumpRewrite { drop_sandbox_line: true, strip_definers: true };

    fn rewritten_in_chunks(rules: DumpRewrite, input: &[u8], chunk: usize) -> Vec<u8> {
        let mut rewriter = DumpRewriter::new(rules);
        let mut out = Vec::new();
        for piece in input.chunks(chunk.max(1)) {
            rewriter.push(piece, &mut out);
        }
        rewriter.finish(&mut out);
        out
    }

    /// The rewrite of `input`, checked to be the same whatever the chunk size.
    fn rewritten(rules: DumpRewrite, input: &str) -> String {
        let whole = rewritten_in_chunks(rules, input.as_bytes(), input.len());
        for chunk in [1, 2, 3, 7, 64] {
            assert_eq!(rewritten_in_chunks(rules, input.as_bytes(), chunk), whole, "chunks of {chunk}");
        }
        String::from_utf8(whole).unwrap()
    }

    #[test]
    fn the_mariadb_sandbox_line_is_dropped_only_at_the_start() {
        let dump = "/*M!999999\\- enable the sandbox mode */ \r\n-- MariaDB dump 10.19\nSELECT 1;\n";
        assert_eq!(rewritten(ALL, dump), "-- MariaDB dump 10.19\nSELECT 1;\n");
        let later = "-- dump\n/*M!999999\\- enable the sandbox mode */\n";
        assert_eq!(rewritten(ALL, later), later, "not the first line: left alone");
        let kept = DumpRewrite { drop_sandbox_line: false, ..ALL };
        assert_eq!(rewritten(kept, dump), dump);
        assert_eq!(rewritten(ALL, "/*M!999999\\- enable the sandbox mode */ SELECT 1;\n"), "SELECT 1;\n", "what follows on the line stays");
    }

    #[test]
    fn definers_in_conditional_comments_are_removed() {
        let view = "/*!50001 CREATE ALGORITHM=UNDEFINED */\n/*!50013 DEFINER=`app`@`%` SQL SECURITY DEFINER */\n/*!50001 VIEW `v` AS select 1 AS `x` */;\n";
        assert_eq!(rewritten(ALL, view), "/*!50001 CREATE ALGORITHM=UNDEFINED */\n/*!50013 SQL SECURITY DEFINER */\n/*!50001 VIEW `v` AS select 1 AS `x` */;\n");
        let trigger = "/*!50003 CREATE*/ /*!50017 DEFINER=`root`@`localhost`*/ /*!50003 TRIGGER `t` BEFORE INSERT ON `users` FOR EACH ROW SET NEW.a = 1 */;;\n";
        assert_eq!(rewritten(ALL, trigger), "/*!50003 CREATE*/ /*!50017 */ /*!50003 TRIGGER `t` BEFORE INSERT ON `users` FOR EACH ROW SET NEW.a = 1 */;;\n");
        let event = "/*!50106 CREATE*/ /*!50117 DEFINER='old'@'10.0.0.%'*/ /*!50106 EVENT `e` ON SCHEDULE EVERY 1 DAY DO SELECT 1 */ ;;\n";
        assert_eq!(rewritten(ALL, event), "/*!50106 CREATE*/ /*!50117 */ /*!50106 EVENT `e` ON SCHEDULE EVERY 1 DAY DO SELECT 1 */ ;;\n");
    }

    #[test]
    fn definers_in_create_headers_are_removed() {
        assert_eq!(rewritten(ALL, "CREATE DEFINER=`root`@`localhost` PROCEDURE `p`()\nBEGIN\nEND ;;\n"), "CREATE PROCEDURE `p`()\nBEGIN\nEND ;;\n");
        assert_eq!(rewritten(ALL, "create definer = 'a''b'@'h' function f() returns int return 1;\r\n"), "create function f() returns int return 1;\r\n");
        assert_eq!(
            rewritten(ALL, "CREATE OR REPLACE ALGORITHM = MERGE DEFINER=CURRENT_USER() SQL SECURITY INVOKER VIEW v AS SELECT 1;\n"),
            "CREATE OR REPLACE ALGORITHM = MERGE SQL SECURITY INVOKER VIEW v AS SELECT 1;\n"
        );
        assert_eq!(rewritten(ALL, "  CREATE DEFINER=`we``ird`@`h\\st` TRIGGER t\n"), "  CREATE TRIGGER t\n");
        assert_eq!(rewritten(ALL, "CREATE DEFINER=app@localhost EVENT e\n"), "CREATE EVENT e\n");
    }

    #[test]
    fn definer_inside_data_or_other_statements_is_left_intact() {
        let untouched = [
            "INSERT INTO `notes` VALUES (1,'CREATE DEFINER=`x`@`y` PROCEDURE'),(2,'/*!50013 DEFINER=`x`@`y` */');\n",
            "/*!50001 VIEW `v` AS select '/*!50013 DEFINER=`x`@`y` */' AS `s` */;\n",
            "/*!40101 SET @saved_cs_client = @@character_set_client */; /*!50013 DEFINER=`x`@`y` */\n",
            "CREATE TABLE `definers` (`DEFINER` varchar(10) DEFAULT 'DEFINER=`x`@`y`');\n",
            "-- CREATE DEFINER=`x`@`y` PROCEDURE p()\n",
            "CREATE DEFINERX=`x`@`y` PROCEDURE p()\n",
            "CREATE DEFINER=`unterminated@`y PROCEDURE p()\n",
            "CREATE ALGORITHM UNDEFINED DEFINER=`x`@`y` VIEW v\n",
            "SQL SECURITY DEFINER\n",
        ];
        for line in untouched {
            assert_eq!(rewritten(ALL, line), line);
        }
        let off = DumpRewrite { strip_definers: false, ..ALL };
        assert_eq!(rewritten(off, "CREATE DEFINER=`root`@`%` PROCEDURE p()\n"), "CREATE DEFINER=`root`@`%` PROCEDURE p()\n");
    }

    #[test]
    fn long_lines_pass_through_with_bounded_memory() {
        let row = format!("INSERT INTO t VALUES ('{}');\n", "é".repeat(LINE_HEAD_BYTES * 3));
        let dump = format!("{row}CREATE DEFINER=`root`@`%` PROCEDURE p()\n{row}");
        let expected = format!("{row}CREATE PROCEDURE p()\n{row}");
        assert_eq!(rewritten_in_chunks(ALL, dump.as_bytes(), 1000), expected.as_bytes());
        let mut rewriter = DumpRewriter::new(ALL);
        let mut out = Vec::new();
        rewriter.push(row.as_bytes(), &mut out);
        assert!(rewriter.head.capacity() <= LINE_HEAD_BYTES, "the head buffer never grows past its limit");
    }

    #[test]
    fn a_last_line_without_line_break_and_empty_input_are_handled() {
        assert_eq!(rewritten(ALL, "CREATE DEFINER=`a`@`b` VIEW v AS SELECT 1"), "CREATE VIEW v AS SELECT 1");
        assert_eq!(rewritten(ALL, ""), "");
        assert_eq!(rewritten(ALL, "\n\n"), "\n\n");
        assert!(!ALL.is_noop() && DumpRewrite::default().is_noop());
    }
}
