//! Markdown to safe HTML for the web browser.
//!
//! Bodies are written by agents, so the output is sanitized: raw HTML is shown
//! as text rather than rendered, and links or images with script-capable
//! schemes are neutralized. Headings get stable ids for the table of
//! contents, tables get a scrolling wrapper, and inline code that is exactly
//! an entry id becomes a link to that entry.

use pulldown_cmark::{CowStr, Event, HeadingLevel, Options, Parser, Tag, TagEnd, html};

/// One heading, for the table of contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocEntry {
    pub level: u8,
    pub text: String,
    pub id: String,
}

pub struct Rendered {
    pub html: String,
    pub toc: Vec<TocEntry>,
}

/// No smart punctuation: bodies are technical, and turning `--flag` into an
/// en dash or curling the quotes in a command would corrupt them.
fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
}

/// Render a body to sanitized HTML and collect its headings.
pub fn render(body: &str) -> Rendered {
    let mut events: Vec<Event> = Vec::new();
    let mut toc = Vec::new();
    let mut used_ids: Vec<String> = Vec::new();
    // Events of the heading being collected, so its id can be set from its text.
    let mut heading: Option<(HeadingLevel, Vec<Event>)> = None;

    for event in Parser::new_ext(body, options()) {
        let event = sanitize(event);
        if let Some((level, buf)) = heading.as_mut() {
            if matches!(event, Event::End(TagEnd::Heading(_))) {
                let level = *level;
                let buf = std::mem::take(buf);
                heading = None;
                let text = plain_text(&buf);
                let id = unique_id(&slug(&text), &mut used_ids);
                toc.push(TocEntry {
                    level: level as u8,
                    text,
                    id: id.clone(),
                });
                events.push(Event::Start(Tag::Heading {
                    level,
                    id: Some(CowStr::from(id)),
                    classes: Vec::new(),
                    attrs: Vec::new(),
                }));
                events.extend(buf);
                events.push(event);
            } else {
                buf.push(event);
            }
            continue;
        }
        match event {
            Event::Start(Tag::Heading { level, .. }) => heading = Some((level, Vec::new())),
            Event::Start(Tag::Table(_)) => {
                events.push(Event::Html(r#"<div class="table-wrap">"#.into()));
                events.push(event);
            }
            Event::End(TagEnd::Table) => {
                events.push(event);
                events.push(Event::Html("</div>".into()));
            }
            Event::Code(code) if is_entry_id(&code) => {
                events.push(Event::Html(
                    format!(r#"<a class="entry-ref" href="/m/{code}"><code>{code}</code></a>"#)
                        .into(),
                ));
            }
            other => events.push(other),
        }
    }

    let mut out = String::with_capacity(body.len() * 3 / 2);
    html::push_html(&mut out, events.into_iter());
    Rendered { html: out, toc }
}

/// A short plain-text preview of a body: headings and Markdown syntax dropped, whitespace
/// collapsed, cut at a word boundary.
pub fn excerpt(body: &str, max_chars: usize) -> String {
    let mut in_heading = false;
    let events: Vec<Event> = Parser::new_ext(body, options())
        .filter(|e| {
            match e {
                Event::Start(Tag::Heading { .. }) => in_heading = true,
                Event::End(TagEnd::Heading(_)) => {
                    in_heading = false;
                    return false;
                }
                _ => {}
            }
            !in_heading
        })
        .collect();
    let text = plain_text(&events);
    if text.chars().count() <= max_chars {
        return text;
    }
    let cut: String = text.chars().take(max_chars).collect();
    let cut = match cut.rfind(' ') {
        Some(i) if i > max_chars / 2 => &cut[..i],
        _ => &cut,
    };
    format!(
        "{}…",
        cut.trim_end_matches(|c: char| c.is_ascii_punctuation() || c == ' ')
    )
}

/// Raw HTML becomes visible text; script-capable link and image targets are dropped.
fn sanitize(event: Event) -> Event {
    match event {
        Event::Html(h) | Event::InlineHtml(h) => Event::Text(h),
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Link {
            link_type,
            dest_url: safe_url(dest_url),
            title,
            id,
        }),
        Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Image {
            link_type,
            dest_url: safe_url(dest_url),
            title,
            id,
        }),
        other => other,
    }
}

fn safe_url(url: CowStr) -> CowStr {
    let lower: String = url
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_control())
        .take(16)
        .collect::<String>()
        .to_ascii_lowercase();
    let scheme = lower.split_once(':').map(|(s, _)| s);
    match scheme {
        Some(s) if !s.contains('/') && !matches!(s, "http" | "https" | "mailto") => {
            CowStr::from("#")
        }
        _ => url,
    }
}

fn plain_text(events: &[Event]) -> String {
    let mut out = String::new();
    for e in events {
        match e {
            Event::Text(t) | Event::Code(t) => out.push_str(t),
            Event::SoftBreak | Event::HardBreak => out.push(' '),
            Event::End(TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item) => out.push(' '),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn slug(text: &str) -> String {
    let mut s = String::new();
    for c in text.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            s.push(c);
        } else if !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
    }
    let s = s.trim_end_matches('-').to_string();
    if s.is_empty() { "section".into() } else { s }
}

fn unique_id(base: &str, used: &mut Vec<String>) -> String {
    let mut id = base.to_string();
    let mut n = 2;
    while used.contains(&id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    used.push(id.clone());
    id
}

/// A MongoDB ObjectId in hex: how entries refer to each other.
pub fn is_entry_id(s: &str) -> bool {
    s.len() == 24
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_gfm() {
        let r = render("| a | b |\n|---|---|\n| 1 | 2 |\n\n- [x] done\n\n~~old~~");
        assert!(
            r.html.contains(r#"<div class="table-wrap"><table>"#),
            "{}",
            r.html
        );
        assert!(r.html.contains("</table>\n</div>"), "{}", r.html);
        assert!(r.html.contains(r#"type="checkbox""#));
        assert!(r.html.contains("<del>old</del>"));
    }

    #[test]
    fn raw_html_is_shown_not_rendered() {
        let r = render("hi <script>alert(1)</script>\n\n<div onclick=x>block</div>");
        assert!(!r.html.contains("<script>"), "{}", r.html);
        assert!(!r.html.contains("<div onclick"), "{}", r.html);
        assert!(r.html.contains("&lt;script&gt;"), "{}", r.html);
    }

    #[test]
    fn dangerous_links_are_neutralized() {
        for md in [
            "[x](javascript:alert(1))",
            "[x](JavaScript:alert(1))",
            "[x](java&#9;script:alert(1))",
            "[x](data:text/html,hi)",
            "![x](vbscript:y)",
        ] {
            let h = render(md).html;
            assert!(
                h.contains(r##"href="#""##) || h.contains(r##"src="#""##),
                "{md} -> {h}"
            );
        }
        for md in [
            "[x](https://example.com/a:b)",
            "[x](/m/abc)",
            "[x](#sec)",
            "[x](mailto:a@b.c)",
        ] {
            assert!(!render(md).html.contains(r##"href="#""##), "{md}");
        }
    }

    #[test]
    fn headings_get_unique_ids_and_toc() {
        let r = render("## Current status\ntext\n## Current status\n### Open `items`");
        assert_eq!(
            r.toc,
            vec![
                TocEntry {
                    level: 2,
                    text: "Current status".into(),
                    id: "current-status".into()
                },
                TocEntry {
                    level: 2,
                    text: "Current status".into(),
                    id: "current-status-2".into()
                },
                TocEntry {
                    level: 3,
                    text: "Open items".into(),
                    id: "open-items".into()
                },
            ]
        );
        assert!(r.html.contains(r#"<h2 id="current-status">"#), "{}", r.html);
        assert!(
            r.html
                .contains(r#"<h3 id="open-items">Open <code>items</code></h3>"#)
        );
    }

    #[test]
    fn entry_ids_in_code_link_to_the_entry() {
        let r = render("see `6af696c9e7f36044a81d5428` and `not-an-id`");
        assert!(r.html.contains(
            r#"<a class="entry-ref" href="/m/6af696c9e7f36044a81d5428"><code>6af696c9e7f36044a81d5428</code></a>"#
        ));
        assert!(r.html.contains("<code>not-an-id</code>"));
    }

    #[test]
    fn excerpt_strips_markdown_and_cuts_on_words() {
        assert_eq!(excerpt("## Head\n**bold** `code`", 100), "bold code");
        assert_eq!(excerpt("one two three four five", 12), "one two…");
    }

    #[test]
    fn punctuation_is_left_alone() {
        let h = render("run --verbose with \"quotes\" and it's").html;
        assert!(h.contains("run --verbose with \"quotes\" and it's"), "{h}");
    }
}
