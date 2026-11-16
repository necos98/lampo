//! The `volumes:` entries of a compose service: short (`./src:/var/www/html:ro`) and long
//! (`type`/`source`/`target`) syntax.

use super::compose::yaml_value;

/// Keys of the long volume syntax: a list item starting with one of them opens a long entry.
const LONG_KEYS: &[&str] = &["type", "source", "target", "read_only", "bind", "volume", "consistency"];

/// One `volumes:` entry of a service.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct Mount {
    /// Host path or volume name, as written.
    pub source: String,
    /// Path inside the container.
    pub target: String,
}

impl Mount {
    /// A `volumes:` list item: a short entry, or the first line of a long one (its other lines
    /// go to [`Self::long_key`]).
    pub fn from_item(item: &str) -> Option<Self> {
        if !LONG_KEYS.iter().any(|key| yaml_value(item, key).is_some()) {
            return Self::parse_short(item);
        }
        let mut mount = Self::default();
        mount.long_key(item);
        Some(mount)
    }

    /// `./src:/var/www/html:ro`, `C:\sites\shop:/app` → (source, target); `/data` (an anonymous
    /// volume) → none.
    pub fn parse_short(entry: &str) -> Option<Self> {
        let mut rest = entry;
        // Trailing options (`:ro`, `:cached`) never start with `/`, the target always does.
        while let Some((head, last)) = rest.rsplit_once(':') {
            if last.starts_with('/') {
                return Some(Self { source: head.to_string(), target: last.to_string() }).filter(|m| !m.source.is_empty());
            }
            rest = head;
        }
        None
    }

    /// A mount of a host folder or file (not a named volume).
    pub fn is_bind(&self) -> bool {
        let source = self.source.as_str();
        source.starts_with(['.', '/', '~', '$', '\\']) || source.as_bytes().get(1) == Some(&b':')
    }

    /// A `source: …` / `target: …` line of the long syntax (other keys are ignored).
    pub fn long_key(&mut self, line: &str) {
        if let Some(source) = yaml_value(line, "source") {
            self.source = source.to_string();
        } else if let Some(target) = yaml_value(line, "target") {
            self.target = target.to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_and_long_entries() {
        let short = |entry: &str| Mount::parse_short(entry).map(|m| (m.source, m.target));
        assert_eq!(short("./app:/var/www/html:ro:cached"), Some(("./app".into(), "/var/www/html".into())));
        assert_eq!(short(r"C:\sites\shop:/app"), Some((r"C:\sites\shop".into(), "/app".into())));
        assert_eq!((short("/var/www/html/vendor"), short(":/app")), (None, None));
        let mut long = Mount::from_item("type: bind").unwrap();
        long.long_key("source: './app'");
        long.long_key("target: /app");
        assert_eq!((long.source.as_str(), long.target.as_str(), long.is_bind()), ("./app", "/app", true));
        assert!(!Mount::from_item("vendor:/var/www/html/vendor").unwrap().is_bind(), "a named volume");
    }
}
