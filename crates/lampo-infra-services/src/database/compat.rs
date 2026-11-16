//! What an import with `--compat` changes on the `mysql` side, and the hint given when a dump
//! that would need it fails without it.

use lampo_domain::PortError;
use lampo_domain::ports::{CommandSpec, MysqlEndpoint};
use lampo_domain::sql_dump::compat::{CompatSuggestion, DumpCompat};

/// Runs `compat`'s session settings before the dump (`--init-command`, one `SET` statement).
pub(super) fn add_session_settings(spec: &mut CommandSpec, endpoint: &MysqlEndpoint, compat: Option<DumpCompat>) {
    if let Some(statement) = compat.and_then(|c| c.init_statement(endpoint.series)) {
        spec.args.push(format!("--init-command={statement}"));
    }
}

/// ` COLLATE …` for a target database created for a dump from `compat` (tables without a
/// collation of their own inherit it); empty otherwise.
pub(super) fn database_collation_clause(compat: Option<DumpCompat>) -> String {
    compat.and_then(DumpCompat::database_collation).map(|c| format!(" COLLATE {c}")).unwrap_or_default()
}

/// Adds to a failed restore the `--compat` its dump's header calls for (only a suggestion: the
/// failure may have another cause).
pub(super) fn with_hint(error: PortError, suggestion: Option<&CompatSuggestion>) -> PortError {
    match (error, suggestion) {
        (PortError::Process { program, message }, Some(suggestion)) => PortError::Process { program, message: format!("{message}. {}", hint(suggestion)) },
        (error, _) => error,
    }
}

fn hint(suggestion: &CompatSuggestion) -> String {
    let origin = match suggestion.compat {
        DumpCompat::Mysql80 => "MySQL 8.0",
        DumpCompat::Mariadb => "MariaDB",
    };
    let version = suggestion.server_version.as_ref().map(|v| format!(" (server version {v})")).unwrap_or_default();
    format!(
        "The dump was made by {origin}{version}: if the error is about foreign keys on non-unique keys, DEFINER accounts, collations or the dump's first line, \
         import it again with --compat {}",
        suggestion.compat
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_process_failure_gets_the_hint_other_errors_do_not() {
        let suggestion = CompatSuggestion { compat: DumpCompat::Mariadb, server_version: Some("10.11.6-MariaDB".into()) };
        let error = with_hint(PortError::process("mysql", "ERROR 1064 (42000) at line 1"), Some(&suggestion));
        let PortError::Process { program, message } = &error else { panic!("{error}") };
        assert_eq!(program, "mysql");
        assert!(message.starts_with("ERROR 1064 (42000) at line 1. The dump was made by MariaDB (server version 10.11.6-MariaDB)"), "{message}");
        assert!(message.ends_with("import it again with --compat mariadb"), "{message}");

        let mysql80 = CompatSuggestion { compat: DumpCompat::Mysql80, server_version: None };
        let error = with_hint(PortError::process("mysql", "ERROR 6125"), Some(&mysql80));
        assert!(error.to_string().contains("made by MySQL 8.0: ") && error.to_string().contains("--compat mysql80"), "{error}");

        assert_eq!(with_hint(PortError::Cancelled, Some(&suggestion)), PortError::Cancelled);
        assert_eq!(with_hint(PortError::process("mysql", "ERROR 1"), None), PortError::process("mysql", "ERROR 1"));
    }

    #[test]
    fn only_mariadb_dumps_change_the_database_collation() {
        assert_eq!(database_collation_clause(Some(DumpCompat::Mariadb)), " COLLATE utf8mb4_general_ci");
        assert_eq!(database_collation_clause(Some(DumpCompat::Mysql80)), "");
        assert_eq!(database_collation_clause(None), "");
    }
}
