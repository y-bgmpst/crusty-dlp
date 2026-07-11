const MAX_MESSAGE_LEN: usize = 4096;

/// Hide query and fragment data by default; those parts commonly contain
/// signed URLs, session identifiers, or access tokens.
pub fn redact_url(value: &str) -> String {
    let trimmed = value.trim();
    let had_fragment = trimmed.contains('#');
    let without_fragment = trimmed.split_once('#').map_or(trimmed, |(base, _)| base);
    let had_query = without_fragment.contains('?');
    let base = without_fragment
        .split_once('?')
        .map_or(without_fragment, |(base, _)| base);
    let redacted = if had_query || had_fragment {
        format!("{base}?[redacted]")
    } else {
        base.to_owned()
    };
    truncate(&redacted, MAX_MESSAGE_LEN)
}

pub fn display_url(value: &str, show_sensitive: bool) -> String {
    if show_sensitive {
        truncate(value, MAX_MESSAGE_LEN)
    } else {
        redact_url(value)
    }
}

/// Redact URL-looking whitespace-delimited tokens in external error text.
pub fn redact_message(value: &str, show_sensitive: bool) -> String {
    let message = if show_sensitive {
        value.to_owned()
    } else {
        value
            .split_whitespace()
            .map(|token| {
                let (url, suffix) = token
                    .strip_suffix(|c: char| ".,;)".contains(c))
                    .map_or((token, ""), |url| (url, &token[url.len()..]));
                if url.starts_with("http://") || url.starts_with("https://") {
                    format!("{}{}", redact_url(url), suffix)
                } else {
                    token.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    };
    truncate(&message, MAX_MESSAGE_LEN)
}

pub fn truncate(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.to_owned();
    }
    let mut output = value[..value.floor_char_boundary(limit.saturating_sub(15))].to_owned();
    output.push_str("… [truncated]");
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_query_and_fragment() {
        assert_eq!(
            redact_url("https://example.test/video?id=abc&token=secret#part"),
            "https://example.test/video?[redacted]"
        );
    }

    #[test]
    fn preserves_full_url_only_when_explicit() {
        let url = "https://example.test/video?token=secret";
        assert_eq!(
            display_url(url, false),
            "https://example.test/video?[redacted]"
        );
        assert_eq!(display_url(url, true), url);
    }

    #[test]
    fn redacts_urls_inside_messages() {
        assert_eq!(
            redact_message("failed https://example.test/v?token=secret.", false),
            "failed https://example.test/v?[redacted]."
        );
    }

    #[test]
    fn bounds_long_messages() {
        let value = redact_message(&"x".repeat(10_000), false);
        assert!(value.len() <= MAX_MESSAGE_LEN);
        assert!(value.ends_with("… [truncated]"));
    }
}
