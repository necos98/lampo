//! Statements of an SQL dump that make it escape the database it is imported into.
//!
//! `mysql --database=shop` only sets the *initial* database: a dump made with
//! `mysqldump --databases` (or `--all-databases`) starts with `CREATE DATABASE` and `USE`
//! statements, and everything after a `USE` runs in the database it names, whatever target the
//! user chose. Such statements are looked for in the head of the dump, where mysqldump writes them.
//!
//! [`compat`] and [`rewrite`] adapt dumps made by other servers (MySQL 8.0, MariaDB) on import.

pub mod compat;
pub mod rewrite;

/// Bytes of a (decompressed) dump inspected before an import.
pub const DUMP_HEAD_BYTES: usize = 64 * 1024;

/// Longest statement excerpt kept for messages (a matching line can be arbitrarily long).
const EXCERPT_CHARS: usize = 120;

/// A statement that leaves the target database: `USE …`, `CREATE DATABASE …`, `CREATE SCHEMA …`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabaseSwitch {
    /// 1-based line of the statement in the dump.
    pub line: usize,
    /// The statement's line, trimmed and shortened to a readable excerpt.
    pub statement: String,
}

/// The first statement of `head` that switches or creates a database, if any. Only line starts
/// are considered: the same words inside comments (`-- use …`), string values or identifiers
/// (`user`, `CREATE TABLE databases`) do not match. Invalid UTF-8 (a cut multi-byte character, a
/// binary blob) is tolerated.
pub fn find_database_switch(head: &[u8]) -> Option<DatabaseSwitch> {
    let text = String::from_utf8_lossy(head);
    text.lines().enumerate().find_map(|(index, line)| {
        let statement = line.trim();
        switches_database(statement).then(|| DatabaseSwitch { line: index + 1, statement: excerpt(statement) })
    })
}

fn switches_database(statement: &str) -> bool {
    let (first, rest) = leading_word(statement);
    if first.eq_ignore_ascii_case("USE") {
        return true;
    }
    let (second, _) = leading_word(rest.trim_start());
    first.eq_ignore_ascii_case("CREATE") && (second.eq_ignore_ascii_case("DATABASE") || second.eq_ignore_ascii_case("SCHEMA"))
}

/// The leading keyword of `text` (ASCII letters and `_`) and what follows it.
fn leading_word(text: &str) -> (&str, &str) {
    let end = text.find(|c: char| !(c.is_ascii_alphabetic() || c == '_')).unwrap_or(text.len());
    text.split_at(end)
}

fn excerpt(statement: &str) -> String {
    match statement.char_indices().nth(EXCERPT_CHARS) {
        Some((cut, _)) => format!("{}…", &statement[..cut]),
        None => statement.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(dump: &str) -> Option<(usize, String)> {
        found_bytes(dump.as_bytes())
    }

    fn found_bytes(dump: &[u8]) -> Option<(usize, String)> {
        find_database_switch(dump).map(|s| (s.line, s.statement))
    }

    #[test]
    fn a_use_statement_is_found_with_its_line() {
        let dump = "-- MySQL dump 10.13\n\n/*!40101 SET NAMES utf8mb4 */;\n  USE `other_db`;\nCREATE TABLE t (id INT);\n";
        assert_eq!(found(dump), Some((4, "USE `other_db`;".into())));
        assert_eq!(found("use other_db;\n"), Some((1, "use other_db;".into())), "case-insensitive");
        assert_eq!(found("USE`x`;"), Some((1, "USE`x`;".into())));
    }

    #[test]
    fn create_database_and_schema_are_found() {
        let dump = "-- Current Database: `shop`\n\nCREATE DATABASE /*!32312 IF NOT EXISTS*/ `shop` /*!40100 DEFAULT CHARACTER SET utf8mb4 */;\n\nUSE `shop`;\n";
        assert_eq!(found(dump).map(|(line, _)| line), Some(3), "the first statement wins");
        assert_eq!(found("create  schema legacy;").map(|(line, _)| line), Some(1));
        assert_eq!(found("CREATE\tDATABASE`x`;").map(|(line, _)| line), Some(1));
    }

    #[test]
    fn a_plain_single_database_dump_does_not_match() {
        let dump = "-- MySQL dump\n-- use the shop database\n# USE nothing\nCREATE TABLE `users` (`user` varchar(10));\nCREATE TABLE databases (id INT);\nINSERT INTO `users` VALUES ('USE `x`;'),('CREATE DATABASE y');\nUPDATE users SET user='a';\nuser_defined;\n";
        assert_eq!(found(dump), None);
        assert_eq!(found(""), None);
    }

    #[test]
    fn invalid_utf8_and_long_lines_are_tolerated() {
        let mut dump = b"INSERT INTO b VALUES (0x".to_vec();
        dump.extend([0xff, 0xfe, 0xc3]);
        dump.extend(b");\nUSE `");
        dump.extend("é".repeat(200).as_bytes());
        dump.extend(b"`;\n");
        let (line, statement) = found_bytes(&dump).unwrap();
        assert_eq!(line, 2);
        assert_eq!(statement.chars().count(), EXCERPT_CHARS + 1);
        assert!(statement.starts_with("USE `éé") && statement.ends_with('…'), "{statement}");
    }
}
