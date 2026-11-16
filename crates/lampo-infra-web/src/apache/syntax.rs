//! Apache configuration syntax helpers (pure).
//!
//! Windows paths are written with forward slashes: Apache accepts them everywhere, and they
//! sidestep two traps — backslash escapes inside quoted arguments (`"C:\dir\"` would escape the
//! closing quote) and `apr_tokenize_to_argv`, which mod_fcgid uses on `FcgidWrapper` and which
//! strips backslashes.

use std::path::Path;

use lampo_domain::{PortError, PortResult};

/// `path` as an Apache path: forward slashes, no `\\?\` prefix, validated so it cannot break
/// out of a quoted argument or trigger `${VAR}` substitution.
pub fn apache_path(path: &Path) -> PortResult<String> {
    let raw = path.to_str().ok_or_else(|| invalid(path, "is not valid Unicode"))?;
    let raw = if let Some(unc) = raw.strip_prefix(r"\\?\UNC\") { format!(r"\\{unc}") } else { raw.strip_prefix(r"\\?\").unwrap_or(raw).to_string() };
    if raw.is_empty() {
        return Err(invalid(path, "is empty"));
    }
    if raw.contains('"') || raw.chars().any(char::is_control) {
        return Err(invalid(path, "contains a quote or a control character"));
    }
    // Apache expands `${NAME}` on every config line and has no escape for it.
    if raw.contains("${") {
        return Err(invalid(path, "contains `${`, which Apache would treat as a variable"));
    }
    let mut out = raw.replace('\\', "/");
    // `C:/` keeps its slash, any other trailing slash goes (`<Directory "C:/site/">` is fine but
    // `DocumentRoot "C:/site/"` triggers a warning).
    while out.len() > 3 && out.ends_with('/') {
        out.pop();
    }
    Ok(out)
}

/// `"<value>"` — the value must already be safe (see [`apache_path`]).
pub fn quoted(value: &str) -> String {
    format!("\"{value}\"")
}

/// `"<value>"` for any text Apache must read back verbatim (`SetEnv NAME "<value>"`): `\` and `"`
/// are escaped the way `ap_getword_conf` unescapes them inside double quotes. Refused when the
/// value contains a control character (a line break would end the directive) or `${`, which
/// Apache expands on every line with no escape for it.
pub fn quoted_text(value: &str) -> PortResult<String> {
    if value.chars().any(|c| c.is_control() && c != '\t') || value.contains("${") {
        return Err(PortError::invalid_data("Apache configuration", "a value contains a control character or `${`"));
    }
    Ok(quoted(&value.replace('\\', "\\\\").replace('"', "\\\"")))
}

/// The argument of `FcgidWrapper`: mod_fcgid splits it with `apr_tokenize_to_argv`, so a path
/// containing spaces needs its own inner quotes, escaped for Apache's quoted-argument parser:
/// `"\"C:/Program Files/php/php-cgi.exe\""`.
pub fn fcgid_wrapper_command(exe: &str) -> String {
    quoted(&argv_word(exe))
}

/// The argument of `ErrorLog` / `CustomLog` sending the log through `rotatelogs`: a circular set
/// of `generations` files (`<file>`, `<file>.1`, …) of up to `size_mib` MiB each. `||` starts the
/// program directly (no `cmd.exe` in between), splitting the line with `apr_tokenize_to_argv`,
/// so paths with spaces get inner quotes as in [`fcgid_wrapper_command`]. Both paths must
/// already be safe (see [`apache_path`]).
///
/// `rotatelogs` reads a `%` in the file name as a `strftime` pattern and refuses it with `-n`:
/// such a file is logged to directly, without rotation, rather than not at all.
pub fn rotated_log(rotatelogs: &str, file: &str, generations: u32, size_mib: u64) -> String {
    if file.contains('%') {
        return quoted(file);
    }
    quoted(&format!("||{} -n {generations} {} {size_mib}M", argv_word(rotatelogs), argv_word(file)))
}

/// One word of an `apr_tokenize_to_argv` command line inside a quoted Apache argument. A value
/// with a space or a single quote (which `apr_tokenize_to_argv` treats as a quote too) is
/// wrapped in escaped double quotes.
fn argv_word(value: &str) -> String {
    if value.contains([' ', '\'']) { format!("\\\"{value}\\\"") } else { value.to_string() }
}

fn invalid(path: &Path, reason: &str) -> PortError {
    PortError::invalid_data("Apache configuration", format!("path `{}` {reason}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_windows_paths() {
        assert_eq!(apache_path(Path::new(r"C:\Program Files\Lampo\php")).unwrap(), "C:/Program Files/Lampo/php");
        assert_eq!(apache_path(Path::new(r"C:\")).unwrap(), "C:/");
        assert_eq!(apache_path(Path::new(r"C:\sites\shop\")).unwrap(), "C:/sites/shop");
        assert_eq!(apache_path(Path::new(r"\\?\C:\x")).unwrap(), "C:/x");
        assert_eq!(apache_path(Path::new(r"\\?\UNC\nas\share\x")).unwrap(), "//nas/share/x");
        assert_eq!(apache_path(Path::new(r"\\nas\share")).unwrap(), "//nas/share");
    }

    #[test]
    fn rejects_unsafe_paths() {
        assert!(apache_path(Path::new("C:\\a\"b")).is_err());
        assert!(apache_path(Path::new("C:\\a${SRVROOT}")).is_err());
        assert!(apache_path(Path::new("C:\\a\nb")).is_err());
        assert!(apache_path(Path::new("")).is_err());
    }

    #[test]
    fn text_quoting_escapes_backslashes_and_quotes() {
        assert_eq!(quoted_text("local").unwrap(), r#""local""#);
        // A trailing backslash is doubled, so it cannot escape the closing quote.
        assert_eq!(quoted_text(r#"a "b" C:\x\"#).unwrap(), r#""a \"b\" C:\\x\\""#);
        assert_eq!(quoted_text("tab\there").unwrap(), "\"tab\there\"");
        assert_eq!(quoted_text("").unwrap(), r#""""#);
        for bad in ["a\nb", "a\rb", "${SRVROOT}", "a\0b"] {
            assert!(quoted_text(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn wrapper_quoting() {
        assert_eq!(fcgid_wrapper_command("C:/php/php-cgi.exe"), "\"C:/php/php-cgi.exe\"");
        assert_eq!(fcgid_wrapper_command("C:/Program Files/php/php-cgi.exe"), r#""\"C:/Program Files/php/php-cgi.exe\"""#);
    }

    #[test]
    fn rotated_log_quoting() {
        assert_eq!(rotated_log("C:/a/bin/rotatelogs.exe", "C:/logs/error.log", 3, 10), r#""||C:/a/bin/rotatelogs.exe -n 3 C:/logs/error.log 10M""#);
        assert_eq!(
            rotated_log("C:/Program Files/a/bin/rotatelogs.exe", "C:/Lampo Data/logs/access.log", 3, 10),
            r#""||\"C:/Program Files/a/bin/rotatelogs.exe\" -n 3 \"C:/Lampo Data/logs/access.log\" 10M""#
        );
        assert_eq!(rotated_log("C:/a/rotatelogs.exe", "C:/Users/o'brien/error.log", 3, 10), r#""||C:/a/rotatelogs.exe -n 3 \"C:/Users/o'brien/error.log\" 10M""#);
        assert_eq!(rotated_log("C:/a/rotatelogs.exe", "C:/Users/100%/error.log", 3, 10), r#""C:/Users/100%/error.log""#, "rotatelogs refuses `%` with -n");
    }
}
