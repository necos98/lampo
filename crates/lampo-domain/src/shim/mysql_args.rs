//! Just enough of the MySQL clients' command-line syntax (`my_getopt`, MySQL 5.7 and 8.x) to
//! tell option values from positional arguments, so the shims can tell whether the user chose a
//! database.
//!
//! Short options can be grouped (`-NBe "SELECT 1"`) and take their value attached (`-uroot`) or
//! as the next argument (`-u root`); `-p` and `-#` take only an attached value. Long options take
//! `--name=value`, or `--name value` when the value is required. `_` and `-` are equivalent in
//! names and `--loose-` is ignored. `--` ends the options. Abbreviated long options are not
//! recognised (8.x rejects them, 5.7 warns).
//!
//! A misreading is harmless when it errs towards "option value": the shim then adds the site's
//! database, which a positional database the client sees overrides anyway.

/// Long options whose value may be the next argument, of `mysql` and `mysqldump` together (no
/// name is value-taking in one client and a flag in the other).
const LONG_WITH_VALUE: &[&str] = &[
    // Connection and option files (both clients).
    "authentication-oci-client-config-profile",
    "bind-address",
    "character-sets-dir",
    "compression-algorithms",
    "connect-timeout",
    "database",
    "default-auth",
    "default-character-set",
    "defaults-extra-file",
    "defaults-file",
    "defaults-group-suffix",
    "dns-srv-name",
    "fido-register-factor",
    "host",
    "init-command",
    "init-command-add",
    "load-data-local-dir",
    "login-path",
    "max-allowed-packet",
    "net-buffer-length",
    "network-namespace",
    "oci-config-file",
    "plugin-authentication-kerberos-client-mode",
    "plugin-dir",
    "port",
    "protocol",
    "register-factor",
    "server-public-key-path",
    "shared-memory-base-name",
    "socket",
    "ssl-ca",
    "ssl-capath",
    "ssl-cert",
    "ssl-cipher",
    "ssl-crl",
    "ssl-crlpath",
    "ssl-fips-mode",
    "ssl-key",
    "ssl-mode",
    "ssl-session-data",
    "tls-ciphersuites",
    "tls-sni-servername",
    "tls-version",
    "user",
    "zstd-compression-level",
    // mysql
    "delimiter",
    "execute",
    "histignore",
    "max-join-size",
    "prompt",
    "select-limit",
    "tee",
    // mysqldump
    "compatible",
    "fields-enclosed-by",
    "fields-escaped-by",
    "fields-optionally-enclosed-by",
    "fields-terminated-by",
    "ignore-error",
    "ignore-table",
    "lines-terminated-by",
    "log-error",
    "mysqld-long-query-time",
    "output-as-version",
    "result-file",
    "set-gtid-purged",
    "tab",
    "where",
];

/// Short options whose value may be the next argument.
pub(super) const MYSQL_SHORT_WITH_VALUE: &str = "DehPSu";
pub(super) const MYSQLDUMP_SHORT_WITH_VALUE: &str = "hPSurTw";

/// Option-file options the clients read only at the very start of the command line.
const LEADING_ONLY: &[&str] = &["--no-defaults", "--print-defaults", "--defaults-file=", "--defaults-extra-file=", "--defaults-group-suffix=", "--login-path="];

/// The arguments sorted by role. Option names are normalised (`default_character_set` →
/// `default-character-set`, no `--loose-`).
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct ParsedArgs<'a> {
    pub long: Vec<String>,
    pub short: Vec<char>,
    pub positional: Vec<&'a str>,
}

impl ParsedArgs<'_> {
    pub fn has_long(&self, name: &str) -> bool {
        self.long.iter().any(|l| l == name)
    }
    pub fn has_short(&self, c: char) -> bool {
        self.short.contains(&c)
    }
}

pub(super) fn parse<'a>(args: &'a [String], short_with_value: &str) -> ParsedArgs<'a> {
    let mut out = ParsedArgs::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == "--" {
            out.positional.extend(it.by_ref().map(String::as_str));
        } else if let Some(long) = arg.strip_prefix("--") {
            let (name, value) = match long.split_once('=') {
                Some((n, _)) => (n, true),
                None => (long, false),
            };
            let name = name.replace('_', "-");
            let name = name.strip_prefix("loose-").map(str::to_string).unwrap_or(name);
            if !value && LONG_WITH_VALUE.contains(&name.as_str()) {
                it.next();
            }
            out.long.push(name);
        } else if let Some(cluster) = arg.strip_prefix('-').filter(|c| !c.is_empty()) {
            for (i, c) in cluster.char_indices() {
                out.short.push(c);
                if short_with_value.contains(c) {
                    if cluster[i + c.len_utf8()..].is_empty() {
                        it.next();
                    }
                    break;
                }
                if c == 'p' || c == '#' {
                    break;
                }
            }
        } else {
            out.positional.push(arg);
        }
    }
    out
}

/// How many leading arguments are option-file options that must stay first.
pub(super) fn leading_only_count(args: &[String]) -> usize {
    args.iter().take_while(|a| LEADING_ONLY.iter().any(|p| if p.ends_with('=') { a.starts_with(p) } else { a.as_str() == *p })).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    fn positional(a: &[&str], short: &str) -> Vec<String> {
        let a = args(a);
        parse(&a, short).positional.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn values_of_short_options_are_not_positional() {
        assert_eq!(positional(&["-u", "root", "-h", "127.0.0.1", "-P", "3307", "shop"], MYSQL_SHORT_WITH_VALUE), ["shop"]);
        assert_eq!(positional(&["-e", "SELECT 1"], MYSQL_SHORT_WITH_VALUE), Vec::<String>::new());
        assert_eq!(positional(&["-NBe", "SELECT 1"], MYSQL_SHORT_WITH_VALUE), Vec::<String>::new());
        assert_eq!(positional(&["-uroot", "-NB", "shop"], MYSQL_SHORT_WITH_VALUE), ["shop"]);
        assert_eq!(positional(&["-D", "blog"], MYSQL_SHORT_WITH_VALUE), Vec::<String>::new());
    }

    #[test]
    fn password_and_debug_take_only_attached_values() {
        assert_eq!(positional(&["-psecret", "shop"], MYSQL_SHORT_WITH_VALUE), ["shop"]);
        assert_eq!(positional(&["-p", "shop"], MYSQL_SHORT_WITH_VALUE), ["shop"]);
        assert_eq!(positional(&["--password", "shop"], MYSQL_SHORT_WITH_VALUE), ["shop"]);
        assert_eq!(positional(&["-#d:t", "shop"], MYSQL_SHORT_WITH_VALUE), ["shop"]);
    }

    #[test]
    fn long_options_with_separate_values() {
        assert_eq!(positional(&["--execute", "SHOW TABLES", "--user", "root"], MYSQL_SHORT_WITH_VALUE), Vec::<String>::new());
        assert_eq!(positional(&["--default_character_set", "utf8mb4", "shop"], MYSQL_SHORT_WITH_VALUE), ["shop"]);
        assert_eq!(positional(&["--loose-database", "blog"], MYSQL_SHORT_WITH_VALUE), Vec::<String>::new());
        assert_eq!(positional(&["--batch", "shop"], MYSQL_SHORT_WITH_VALUE), ["shop"], "flags take no value");
        assert_eq!(positional(&["--table", "shop"], MYSQL_SHORT_WITH_VALUE), ["shop"]);
        let a = args(&["--loose-database=blog"]);
        assert!(parse(&a, MYSQL_SHORT_WITH_VALUE).has_long("database"));
    }

    #[test]
    fn double_dash_ends_the_options() {
        assert_eq!(positional(&["-e", "SELECT 1", "--", "-weird-db"], MYSQL_SHORT_WITH_VALUE), ["-weird-db"]);
        assert_eq!(positional(&["--"], MYSQL_SHORT_WITH_VALUE), Vec::<String>::new());
    }

    #[test]
    fn short_options_differ_per_client() {
        // `-e` is `--execute` for mysql, `--extended-insert` (a flag) for mysqldump.
        assert_eq!(positional(&["-e", "shop"], MYSQLDUMP_SHORT_WITH_VALUE), ["shop"]);
        assert_eq!(positional(&["-r", "out.sql", "-w", "id<10", "shop", "orders"], MYSQLDUMP_SHORT_WITH_VALUE), ["shop", "orders"]);
        assert_eq!(positional(&["--result-file", "out.sql", "--ignore-table", "shop.logs"], MYSQLDUMP_SHORT_WITH_VALUE), Vec::<String>::new());
    }

    #[test]
    fn option_files_options_are_found_at_the_start_only() {
        assert_eq!(leading_only_count(&args(&["--no-defaults", "--login-path=local", "-e", "x"])), 2);
        assert_eq!(leading_only_count(&args(&["--defaults-file=C:/my.ini"])), 1);
        assert_eq!(leading_only_count(&args(&["-e", "x", "--no-defaults"])), 0);
        assert_eq!(leading_only_count(&args(&["--no-defaults-please"])), 0);
    }
}
