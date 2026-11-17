//! How the custom directives nest their sections, as Apache reads them (logical lines).
//!
//! Every vhost ends with Lampo's own `<If>`, which denies the project's internal files
//! (`.env*`, `.git`, `.ht*`, logs, dumps) after any custom `<Location>` or `<If>` has merged.
//! That works because Apache merges `<If>` sections after every other section, in order, but
//! walks each nesting level only after the previous one. An `<If>` nested inside another would
//! therefore merge after Lampo's barrier and could re-grant those files, so nesting is refused.

use super::{LogicalLine, directive_words};

/// The sections Apache evaluates per request and merges after every other one.
const CONDITIONAL_SECTIONS: &[&str] = &["if", "elseif", "else"];

const NESTED_CONDITION: &str = "it is nested inside another <If>, <ElseIf> or <Else>: Apache applies nested conditions after Lampo's protection of the project's internal files (.env, .git, …), so they could expose them (combine the conditions with `&&` instead)";

/// `Err((line, reason))` for the first `<If>`, `<ElseIf>` or `<Else>` opened inside another one.
/// Closing tags pop the innermost open section; an unbalanced text is left to `httpd -t`.
pub(super) fn check_nesting(lines: &[LogicalLine<'_>]) -> Result<(), (usize, String)> {
    let mut open: Vec<String> = Vec::new();
    for line in lines {
        let Some(words) = directive_words(&line.text) else { continue };
        let raw_name = &words[0];
        let name = raw_name.to_ascii_lowercase();
        if name.starts_with("</") {
            open.pop();
        } else if let Some(section) = name.strip_prefix('<') {
            if is_conditional(section) && open.iter().any(|outer| is_conditional(outer)) {
                return Err((line.number, format!("`{raw_name}` is not allowed: {NESTED_CONDITION}")));
            }
            open.push(section.to_string());
        }
    }
    Ok(())
}

fn is_conditional(section: &str) -> bool {
    CONDITIONAL_SECTIONS.contains(&section)
}

#[cfg(test)]
mod tests {
    use crate::error::DomainError;
    use crate::project::ApacheDirectives;

    fn refused(text: &str) -> String {
        match ApacheDirectives::new(text) {
            Err(DomainError::InvalidApacheDirectives(reason)) => reason,
            other => panic!("`{text}` should be refused, got {other:?}"),
        }
    }

    #[test]
    fn a_condition_nested_in_another_is_refused() {
        for text in [
            "<If \"true\">\n    <If \"true\">\n        Require local\n    </If>\n</If>",
            "<Location />\n  <If \"true\">\n    <ElseIf \"false\">\n    </ElseIf>\n  </If>\n</Location>",
            "<If \"-n %{HTTP_HOST}\">\n</If>\n<Else>\n  <IfModule headers_module>\n    <If \"true\">\n    </If>\n  </IfModule>\n</Else>",
            "<ElseIf \"true\">\n<Else>\n</Else>\n</ElseIf>",
            "<if 'true'>\n\"<IF\" 'true'>\n</if>\n</if>",
        ] {
            assert!(refused(text).contains("nested inside another <If>"), "{text}: {}", refused(text));
        }
    }

    #[test]
    fn the_error_names_the_line_of_the_nested_section() {
        let reason = refused("Header set X 1\n<If \"true\">\n    <Else>\n    </Else>\n</If>");
        assert!(reason.starts_with("line 3: `<Else` is not allowed: it is nested inside another <If>"), "{reason}");
    }

    #[test]
    fn a_line_continuation_is_read_as_apache_reads_it() {
        // Apache sees one `<If "true">` here, opened on line 2: the nested one is still found.
        assert!(refused("<If \"true\">\n<If \\\n\"true\">\n</If>\n</If>").starts_with("line 2:"));
        // A `<If` that is only the argument of a continued line is not a section to Apache.
        assert!(ApacheDirectives::new("<If \"true\">\nHeader set X \\\n<If\n</If>").is_ok());
    }

    #[test]
    fn conditions_side_by_side_or_inside_other_sections_are_accepted() {
        for text in [
            "<If \"%{HTTP_HOST} == 'a'\">\n    Header set X a\n</If>\n<ElseIf \"%{HTTP_HOST} == 'b'\">\n    Header set X b\n</ElseIf>\n<Else>\n    Header set X c\n</Else>",
            "<Location /admin>\n    <If \"%{QUERY_STRING} =~ /debug/\">\n        Require local\n    </If>\n</Location>",
            "<IfModule headers_module>\n    <If \"true\">\n        Header set X 1\n    </If>\n</IfModule>\n<If \"true\">\n</If>",
            "<Files x.txt>\n    <If \"true\">\n    </If>\n</Files>\n# <If \"true\"> in a comment",
        ] {
            assert!(ApacheDirectives::new(text).is_ok(), "{text}: {:?}", ApacheDirectives::new(text));
        }
    }
}
