//! Statement rendering: Markdown in, sanitized HTML out.
//!
//! Problem authors are semi-trusted, and a stored XSS in a statement reaches
//! every learner who opens the page. Two layers, in order: raw HTML blocks are
//! dropped at the parser (never rendered at all), and the rendered output is
//! run through an allowlist sanitizer. Render then sanitize; the renderer's
//! own escaping is not trusted alone.

use std::sync::OnceLock;

use ammonia::Builder;
use pulldown_cmark::{Event, Options, Parser};

/// The HTML elements a statement may carry: headings, paragraphs, lists,
/// code, tables, links, emphasis, blockquotes. Nothing that scripts, embeds,
/// styles, or forms.
const ALLOWED_TAGS: &[&str] = &[
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "p",
    "ul",
    "ol",
    "li",
    "code",
    "pre",
    "table",
    "thead",
    "tbody",
    "tr",
    "th",
    "td",
    "a",
    "blockquote",
    "strong",
    "em",
    "hr",
    "br",
];

fn sanitizer() -> &'static Builder<'static> {
    static SANITIZER: OnceLock<Builder<'static>> = OnceLock::new();
    SANITIZER.get_or_init(|| {
        let mut builder = Builder::empty();
        builder
            .tags(ALLOWED_TAGS.iter().copied().collect())
            .link_rel(Some("noopener noreferrer"))
            .url_schemes(["http", "https"].into_iter().collect());
        builder
    })
}

/// Render a statement to HTML safe to serve to every learner.
pub fn render_statement(markdown: &str) -> String {
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;

    // Drop raw HTML at the source: a `<script>` block in the Markdown never
    // even reaches the sanitizer.
    let events = Parser::new_ext(markdown, options)
        .filter(|event| !matches!(event, Event::Html(_) | Event::InlineHtml(_)));

    let mut html = String::with_capacity(markdown.len() * 2);
    pulldown_cmark::html::push_html(&mut html, events);

    sanitizer().clean(&html).to_string()
}

/// Whether a README qualifies as a statement: non-empty and carrying at least
/// one heading.
pub fn is_valid_statement(markdown: &str) -> bool {
    if markdown.trim().is_empty() {
        return false;
    }
    let options = Options::ENABLE_TABLES;
    Parser::new_ext(markdown, options)
        .any(|event| matches!(event, Event::Start(pulldown_cmark::Tag::Heading { .. })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_blocks_are_dropped_entirely() {
        let html = render_statement("# Title\n\n<script>alert(1)</script>\n\nBody.");
        assert!(!html.contains("script"), "{html}");
        assert!(!html.contains("alert"), "{html}");
        assert!(html.contains("<h1>"));
        assert!(html.contains("Body."));
    }

    #[test]
    fn event_handler_attributes_are_stripped() {
        let html = render_statement("Look: <img src=x onerror=alert(1)> here");
        assert!(!html.contains("onerror"), "{html}");
        assert!(!html.contains("<img"), "{html}");
        assert!(html.contains("here"));
    }

    #[test]
    fn javascript_urls_are_refused() {
        let html = render_statement("[click](javascript:alert(1))");
        assert!(!html.contains("javascript:"), "{html}");
    }

    #[test]
    fn links_get_noopener() {
        let html = render_statement("[docs](https://example.com)");
        assert!(html.contains("rel=\"noopener noreferrer\""), "{html}");
        assert!(html.contains("href=\"https://example.com\""), "{html}");
    }

    #[test]
    fn the_allowlist_keeps_what_statements_need() {
        let markdown = "\
# Spend a P2WPKH output

A paragraph with `inline code` and **bold**.

```rust
fn main() {}
```

| input | output |
|-------|--------|
| a     | b      |

- one
- two
";
        let html = render_statement(markdown);
        for needle in ["<h1>", "<code>", "<pre>", "<table>", "<li>", "<strong>"] {
            assert!(html.contains(needle), "missing {needle} in {html}");
        }
    }

    #[test]
    fn statement_validity_needs_content_and_a_heading() {
        assert!(is_valid_statement("# Title\n\nBody"));
        assert!(!is_valid_statement("   \n\n  "));
        assert!(!is_valid_statement("just a paragraph, no heading"));
    }
}
