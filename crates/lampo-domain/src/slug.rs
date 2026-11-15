//! Slugs: the identifier rules shared by project, preset and worktree ids, and the conversion of
//! any display name into one.

use crate::error::DomainError;

/// Longest slug: one DNS label, since slugs become host names (`<slug>.localhost`).
pub(crate) const MAX_SLUG_LEN: usize = 63;

pub(crate) fn validate_slug(s: &str) -> Result<String, DomainError> {
    let ok = !s.is_empty()
        && s.len() <= MAX_SLUG_LEN
        && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !s.starts_with('-')
        && !s.ends_with('-')
        && !s.contains("--");
    if ok { Ok(s.to_string()) } else { Err(DomainError::InvalidSlug(s.to_string())) }
}

/// Turns any display name into a valid slug (`"Il mio Shop!"` → `"il-mio-shop"`,
/// `"Straße"` → `"strasse"`). Latin letters with diacritics are transliterated, every other
/// run of characters becomes one dash. Never fails: falls back to `fallback` when nothing usable
/// is left.
pub fn slugify(name: &str, fallback: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars().flat_map(char::to_lowercase).filter(|c| !is_combining_mark(*c)) {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if let Some(latin) = transliterate(c) {
            out.push_str(latin);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let mut s = out.trim_matches('-').to_string();
    s.truncate(MAX_SLUG_LEN);
    let s = s.trim_end_matches('-').to_string();
    if s.is_empty() { fallback.to_string() } else { s }
}

/// Combining diacritical marks (U+0300–U+036F): the accents of decomposed text (`e` + `◌́`),
/// dropped so the base letter stays.
fn is_combining_mark(c: char) -> bool {
    ('\u{0300}'..='\u{036F}').contains(&c)
}

/// ASCII spelling of a lowercase Latin letter with a diacritic or a ligature.
fn transliterate(c: char) -> Option<&'static str> {
    let latin = match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => "a",
        'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => "c",
        'ď' | 'đ' | 'ð' => "d",
        'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => "e",
        'ĝ' | 'ğ' | 'ġ' | 'ģ' => "g",
        'ĥ' | 'ħ' => "h",
        'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => "i",
        'ĵ' => "j",
        'ķ' => "k",
        'ĺ' | 'ļ' | 'ľ' | 'ŀ' | 'ł' => "l",
        'ñ' | 'ń' | 'ņ' | 'ň' => "n",
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => "o",
        'ŕ' | 'ŗ' | 'ř' => "r",
        'ś' | 'ŝ' | 'ş' | 'š' | 'ș' => "s",
        'ţ' | 'ť' | 'ŧ' | 'ț' => "t",
        'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => "u",
        'ŵ' => "w",
        'ý' | 'ÿ' | 'ŷ' => "y",
        'ź' | 'ż' | 'ž' => "z",
        'ß' => "ss",
        'æ' => "ae",
        'œ' => "oe",
        'þ' => "th",
        'ĳ' => "ij",
        _ => return None,
    };
    Some(latin)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_names() {
        assert_eq!(slugify("Il mio Shop!", "x"), "il-mio-shop");
        assert_eq!(slugify("  Città -- Nuova ", "x"), "citta-nuova");
        assert_eq!(slugify("!!!", "project"), "project");
        assert_eq!(slugify("日本語", "worktree"), "worktree");
    }

    #[test]
    fn slugify_transliterates_latin_letters() {
        assert_eq!(slugify("Façade", "x"), "facade");
        assert_eq!(slugify("Straße", "x"), "strasse");
        assert_eq!(slugify("STRAẞE", "x"), "strasse");
        assert_eq!(slugify("España Ñandú", "x"), "espana-nandu");
        assert_eq!(slugify("Søren Łódź Ålborg", "x"), "soren-lodz-alborg");
        assert_eq!(slugify("Æsir Œuvre", "x"), "aesir-oeuvre");
        assert_eq!(slugify("Müller Öl Äpfel", "x"), "muller-ol-apfel");
        assert_eq!(slugify("Ærø-Þórr", "x"), "aero-thorr");
        assert_eq!(slugify("Şişli Čapek Ğ", "x"), "sisli-capek-g");
        assert_eq!(slugify("İstanbul", "x"), "istanbul", "the combining dot of a lowercased `İ` is dropped");
        assert_eq!(slugify("Cafe\u{0301} cre\u{0300}me", "x"), "cafe-creme", "decomposed accents");
    }

    #[test]
    fn slugify_output_is_always_a_valid_slug() {
        let slug = slugify(&"ß".repeat(40), "x");
        assert_eq!(slug, "s".repeat(MAX_SLUG_LEN), "cut at the slug limit");
        for name in ["Façade", "--a--", "a ß b", "Ærø", "x\u{0301}"] {
            assert!(validate_slug(&slugify(name, "fallback")).is_ok(), "{name}");
        }
    }

    #[test]
    fn slug_rules() {
        assert!(validate_slug("bad id").is_err());
        assert!(validate_slug("a--b").is_err());
        assert!(validate_slug("-a").is_err());
        assert!(validate_slug(&"a".repeat(MAX_SLUG_LEN + 1)).is_err());
        assert!(validate_slug(&"a".repeat(MAX_SLUG_LEN)).is_ok());
    }
}
