//! Front matter parsing

use std::collections::HashMap;

/// Parsed front matter of an MDX/Markdown document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrontMatter {
    pub title: Option<String>,
    pub template: Option<String>,
    pub meta: HashMap<String, String>,
}

/// Split `---` front matter from the body.
///
/// Supported keys: `title`, `template`, and any other `key: value` lines are
/// collected into `meta` (including `description`).
pub fn parse_front_matter(source: &str) -> (FrontMatter, String) {
    let mut front_matter = FrontMatter::default();
    let trimmed = source.strip_prefix('\u{feff}').unwrap_or(source);

    let Some(rest) = trimmed
        .strip_prefix("---\n")
        .or_else(|| trimmed.strip_prefix("---\r\n"))
    else {
        return (front_matter, source.to_string());
    };

    let mut offset = 0usize;
    let mut body_start = None;
    for raw in rest.split_inclusive('\n') {
        let line = raw.trim_end_matches(['\n', '\r']);
        if line.trim() == "---" {
            body_start = Some(offset + raw.len());
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim().to_string();
            let value = value.trim().trim_matches('"').to_string();
            match key.as_str() {
                "title" => front_matter.title = Some(value),
                "template" => front_matter.template = Some(value),
                _ => {
                    front_matter.meta.insert(key, value);
                }
            }
        }
        offset += raw.len();
    }

    match body_start {
        Some(index) => (front_matter, rest[index..].to_string()),
        // An opening `---` with no closing one is not front matter: returning
        // the partial metadata together with the *full* source handed callers a
        // pair that disagreed (metadata from lines the body still contained).
        // Treat the whole document as the body (WS11).
        None => (FrontMatter::default(), source.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_metadata_from_the_body() {
        let source = "---\ntitle: Hello\ntemplate: page\ndescription: A page\n---\nBody";
        let (front_matter, body) = parse_front_matter(source);
        assert_eq!(front_matter.title.as_deref(), Some("Hello"));
        assert_eq!(front_matter.template.as_deref(), Some("page"));
        assert_eq!(
            front_matter.meta.get("description").map(String::as_str),
            Some("A page")
        );
        assert_eq!(body, "Body");
    }

    #[test]
    fn a_document_without_front_matter_is_all_body() {
        let (front_matter, body) = parse_front_matter("Just a body");
        assert_eq!(front_matter, FrontMatter::default());
        assert_eq!(body, "Just a body");
    }

    /// WS11: an unterminated block is not front matter — metadata and body must
    /// agree, so both come back untouched.
    #[test]
    fn an_unterminated_block_is_not_metadata() {
        let source = "---\ntitle: Hello\nBody without a closing fence";
        let (front_matter, body) = parse_front_matter(source);
        assert_eq!(front_matter, FrontMatter::default());
        assert_eq!(body, source);
    }

    #[test]
    fn handles_a_bom_and_crlf() {
        let source = "\u{feff}---\r\ntitle: Hello\r\n---\r\nBody";
        let (front_matter, body) = parse_front_matter(source);
        assert_eq!(front_matter.title.as_deref(), Some("Hello"));
        assert_eq!(body, "Body");
    }

    #[test]
    fn a_closing_fence_with_trailing_spaces_still_closes() {
        let source = "---\ntitle: Hello\n---   \nBody";
        let (front_matter, body) = parse_front_matter(source);
        assert_eq!(front_matter.title.as_deref(), Some("Hello"));
        assert_eq!(body, "Body");
    }
}
