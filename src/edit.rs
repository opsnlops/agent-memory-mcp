//! Targeted body edits and Markdown section extraction.
//!
//! Agents edit long entries by quoting the text they want to change instead
//! of resending the whole body, the same model as an editor's exact-match
//! "replace" tool. Everything here is pure so it can be re-applied to a fresh
//! body when another session wrote first.

use std::fmt;

/// One exact-text replacement.
#[derive(Debug, Clone)]
pub struct TextEdit {
    pub old_text: String,
    pub new_text: String,
    pub replace_all: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum EditError {
    NothingToDo,
    EmptyOldText { index: usize },
    NotFound { index: usize },
    Ambiguous { index: usize, count: usize },
    EmptyResult,
}

impl fmt::Display for EditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EditError::NothingToDo => write!(f, "pass at least one edit or append"),
            EditError::EmptyOldText { index } => {
                write!(f, "edits[{index}]: old_text must not be empty")
            }
            EditError::NotFound { index } => write!(
                f,
                "edits[{index}]: old_text not found in the body. It must match exactly, \
                 including whitespace and line breaks; `get` the entry (or one section) \
                 and copy the text"
            ),
            EditError::Ambiguous { index, count } => write!(
                f,
                "edits[{index}]: old_text matches {count} places; include more surrounding \
                 text so it is unique, or set replace_all"
            ),
            EditError::EmptyResult => write!(f, "the edits would leave the body empty"),
        }
    }
}

impl std::error::Error for EditError {}

/// Apply `edits` in order, then `append`, returning the new body and how many
/// replacements each edit made. All-or-nothing: any failing edit fails the lot.
pub fn apply(
    body: &str,
    edits: &[TextEdit],
    append: Option<&str>,
) -> Result<(String, Vec<usize>), EditError> {
    let append = append.filter(|a| !a.trim().is_empty());
    if edits.is_empty() && append.is_none() {
        return Err(EditError::NothingToDo);
    }
    let mut out = body.to_string();
    let mut counts = Vec::with_capacity(edits.len());
    for (index, e) in edits.iter().enumerate() {
        if e.old_text.is_empty() {
            return Err(EditError::EmptyOldText { index });
        }
        let count = out.matches(e.old_text.as_str()).count();
        match count {
            0 => return Err(EditError::NotFound { index }),
            1 => out = out.replacen(e.old_text.as_str(), &e.new_text, 1),
            n if e.replace_all => {
                out = out.replace(e.old_text.as_str(), &e.new_text);
                debug_assert!(n > 1);
            }
            n => return Err(EditError::Ambiguous { index, count: n }),
        }
        counts.push(count);
    }
    if let Some(text) = append {
        let trimmed = out.trim_end_matches('\n').len();
        out.truncate(trimmed);
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(text.trim_end_matches('\n'));
        out.push('\n');
    }
    if out.trim().is_empty() {
        return Err(EditError::EmptyResult);
    }
    Ok((out, counts))
}

/// A Markdown ATX heading line: its level and text.
fn heading(line: &str) -> Option<(usize, &str)> {
    let level = line.bytes().take_while(|&b| b == b'#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let rest = &line[level..];
    if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
        return None;
    }
    Some((level, rest.trim().trim_end_matches('#').trim()))
}

/// Every heading in the body, as written (`## Current status`), skipping
/// fenced code blocks so a `# comment` in a shell snippet isn't a heading.
pub fn headings(body: &str) -> Vec<String> {
    lines_outside_fences(body)
        .filter(|(_, line)| heading(line).is_some())
        .map(|(_, line)| line.trim_end().to_string())
        .collect()
}

fn lines_outside_fences(body: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut in_fence = false;
    let mut offset = 0;
    body.split_inclusive('\n').filter_map(move |raw| {
        let start = offset;
        offset += raw.len();
        let line = raw.trim_end_matches(['\n', '\r']);
        if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
            in_fence = !in_fence;
            return None;
        }
        (!in_fence).then_some((start, line))
    })
}

#[derive(Debug, PartialEq, Eq)]
pub enum SectionError {
    NotFound,
    Ambiguous(Vec<String>),
}

/// The section under the heading named `wanted`, from its heading line up to
/// the next heading of the same or a higher level. `wanted` matches the
/// heading text case-insensitively, with or without the leading `#`s; an
/// exact match wins, otherwise a unique prefix match is accepted.
pub fn section<'a>(body: &'a str, wanted: &str) -> Result<&'a str, SectionError> {
    let wanted = heading(wanted.trim())
        .map(|(_, t)| t)
        .unwrap_or(wanted.trim())
        .to_lowercase();
    let all: Vec<(usize, usize, &str)> = lines_outside_fences(body)
        .filter_map(|(start, line)| heading(line).map(|(lvl, text)| (start, lvl, text)))
        .collect();
    let exact: Vec<_> = all
        .iter()
        .filter(|(_, _, t)| t.to_lowercase() == wanted)
        .collect();
    let candidates = if exact.is_empty() {
        all.iter()
            .filter(|(_, _, t)| t.to_lowercase().starts_with(&wanted))
            .collect()
    } else {
        exact
    };
    let &&(start, level, _) = match candidates.as_slice() {
        [] => return Err(SectionError::NotFound),
        [one] => one,
        many => {
            return Err(SectionError::Ambiguous(
                many.iter().map(|(_, _, t)| t.to_string()).collect(),
            ));
        }
    };
    let end = all
        .iter()
        .find(|(s, lvl, _)| *s > start && *lvl <= level)
        .map(|(s, _, _)| *s)
        .unwrap_or(body.len());
    Ok(&body[start..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(old: &str, new: &str) -> TextEdit {
        TextEdit {
            old_text: old.into(),
            new_text: new.into(),
            replace_all: false,
        }
    }

    #[test]
    fn replaces_a_unique_match() {
        let (out, counts) = apply("a b c", &[e("b", "B")], None).unwrap();
        assert_eq!(out, "a B c");
        assert_eq!(counts, vec![1]);
    }

    #[test]
    fn edits_apply_in_order_and_see_earlier_results() {
        let (out, _) = apply("one", &[e("one", "two"), e("two", "three")], None).unwrap();
        assert_eq!(out, "three");
    }

    #[test]
    fn missing_text_fails_the_whole_batch() {
        let err = apply("a b", &[e("a", "A"), e("zzz", "y")], None).unwrap_err();
        assert_eq!(err, EditError::NotFound { index: 1 });
    }

    #[test]
    fn ambiguous_match_needs_replace_all() {
        let err = apply("x x x", &[e("x", "y")], None).unwrap_err();
        assert_eq!(err, EditError::Ambiguous { index: 0, count: 3 });
        let all = TextEdit {
            replace_all: true,
            ..e("x", "y")
        };
        let (out, counts) = apply("x x x", &[all], None).unwrap();
        assert_eq!(out, "y y y");
        assert_eq!(counts, vec![3]);
    }

    #[test]
    fn empty_old_text_and_no_op_are_rejected() {
        assert_eq!(
            apply("a", &[e("", "b")], None).unwrap_err(),
            EditError::EmptyOldText { index: 0 }
        );
        assert_eq!(apply("a", &[], None).unwrap_err(), EditError::NothingToDo);
        assert_eq!(
            apply("a", &[], Some("  ")).unwrap_err(),
            EditError::NothingToDo
        );
    }

    #[test]
    fn deleting_everything_is_rejected() {
        assert_eq!(
            apply("only", &[e("only", "")], None).unwrap_err(),
            EditError::EmptyResult
        );
    }

    #[test]
    fn append_separates_with_one_blank_line() {
        let (out, _) = apply("body\n\n\n", &[], Some("more\n")).unwrap();
        assert_eq!(out, "body\n\nmore\n");
        let (out, _) = apply("body", &[], Some("more")).unwrap();
        assert_eq!(out, "body\n\nmore\n");
    }

    const DOC: &str =
        "intro\n## Status\nok\n### Detail\nd\n```sh\n# not a heading\n```\n## Decisions\nx\n";

    #[test]
    fn section_runs_to_next_heading_of_same_level() {
        assert_eq!(
            section(DOC, "status").unwrap(),
            "## Status\nok\n### Detail\nd\n```sh\n# not a heading\n```\n"
        );
        assert_eq!(section(DOC, "## Decisions").unwrap(), "## Decisions\nx\n");
        assert_eq!(
            section(DOC, "Detail").unwrap(),
            "### Detail\nd\n```sh\n# not a heading\n```\n"
        );
    }

    #[test]
    fn section_prefix_match_and_errors() {
        assert_eq!(section(DOC, "Dec").unwrap(), "## Decisions\nx\n");
        assert_eq!(section(DOC, "nope"), Err(SectionError::NotFound));
        assert_eq!(section(DOC, "not a heading"), Err(SectionError::NotFound));
        assert!(matches!(section(DOC, "D"), Err(SectionError::Ambiguous(v)) if v.len() == 2));
    }

    #[test]
    fn headings_skip_code_fences() {
        assert_eq!(
            headings(DOC),
            vec!["## Status", "### Detail", "## Decisions"]
        );
    }

    #[test]
    fn hash_without_space_is_not_a_heading() {
        assert_eq!(headings("#hashtag\n####### seven\n# real"), vec!["# real"]);
    }
}
