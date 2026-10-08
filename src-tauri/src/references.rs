//! Markdown destinations with exact source ranges. Structural rewrites can
//! change only the destination, keeping labels, titles, aliases and examples.
use pulldown_cmark::{Event, LinkType, Options, Parser, Tag};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, ops::Range};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReferenceKind {
    Markdown,
    Wiki,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    pub destination: String,
    pub destination_range: Range<usize>,
    pub source_range: Range<usize>,
    pub label: String,
    pub kind: ReferenceKind,
    pub image: bool,
    pub definition: bool,
}

fn bracket_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut index = start;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => {
                index += 2;
                continue;
            }
            b'`' => {
                let run = bytes[index..]
                    .iter()
                    .take_while(|&&byte| byte == b'`')
                    .count();
                let mut end = index + run;
                let mut found = false;
                while end < bytes.len() {
                    if bytes[end] == b'`' {
                        let length = bytes[end..]
                            .iter()
                            .take_while(|&&byte| byte == b'`')
                            .count();
                        if length == run {
                            index = end + run;
                            found = true;
                            break;
                        }
                        end += length;
                    } else {
                        end += 1;
                    }
                }
                if found {
                    continue;
                }
                index += run;
                continue;
            }
            b'[' => depth += 1,
            b']' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn destination_range(raw: &str, mut start: usize) -> Option<Range<usize>> {
    let bytes = raw.as_bytes();
    while bytes.get(start).is_some_and(u8::is_ascii_whitespace) {
        start += 1;
    }
    if bytes.get(start) == Some(&b'<') {
        let from = start + 1;
        let mut end = from;
        while end < bytes.len() {
            if bytes[end] == b'\\' {
                end += 2;
                continue;
            }
            if bytes[end] == b'>' {
                return Some(from..end);
            }
            end += 1;
        }
        return None;
    }
    let mut end = start;
    let mut depth = 0usize;
    while end < bytes.len() {
        match bytes[end] {
            b'\\' => {
                end += 2;
                continue;
            }
            b'(' => depth += 1,
            b')' if depth > 0 => depth -= 1,
            b')' => break,
            byte if byte.is_ascii_whitespace() => break,
            _ => {}
        }
        end += 1;
    }
    (end > start).then_some(start..end)
}

fn definition_destination(content: &str, span: &Range<usize>) -> Option<Range<usize>> {
    let raw = content.get(span.clone())?;
    let opening = raw.find('[')?;
    let closing = bracket_end(raw.as_bytes(), opening)?;
    if raw.as_bytes().get(closing + 1) != Some(&b':') {
        return None;
    }
    let range = destination_range(raw, closing + 2)?;
    Some(span.start + range.start..span.start + range.end)
}

pub fn scan(content: &str) -> Vec<Reference> {
    let mut parser = Parser::new_ext(
        content,
        Options::ENABLE_WIKILINKS | Options::ENABLE_TABLES | Options::ENABLE_FOOTNOTES,
    )
    .into_offset_iter();
    let definitions: BTreeMap<_, _> = parser
        .reference_definitions()
        .iter()
        .filter_map(|(label, definition)| {
            Some((
                label.to_string(),
                Reference {
                    destination: definition.dest.to_string(),
                    destination_range: definition_destination(content, &definition.span)?,
                    source_range: definition.span.clone(),
                    label: String::new(),
                    kind: ReferenceKind::Markdown,
                    image: false,
                    definition: true,
                },
            ))
        })
        .collect();
    let mut references: Vec<_> = definitions.values().cloned().collect();
    while let Some((event, source_range)) = parser.next() {
        let (link_type, destination, id, image) = match event {
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                id,
                ..
            }) => (link_type, dest_url, id, false),
            Event::Start(Tag::Image {
                link_type,
                dest_url,
                id,
                ..
            }) => (link_type, dest_url, id, true),
            _ => continue,
        };
        let Some(raw) = content.get(source_range.clone()) else {
            continue;
        };
        let Some(opening) = raw.find('[') else {
            continue;
        };
        if matches!(link_type, LinkType::WikiLink { .. }) {
            let Some(inner) = raw
                .strip_prefix('!')
                .unwrap_or(raw)
                .strip_prefix("[[")
                .and_then(|raw| raw.strip_suffix("]]"))
            else {
                continue;
            };
            let target = inner.split('|').next().unwrap_or("");
            let leading = target.len() - target.trim_start().len();
            let range = source_range.start + opening + 2 + leading
                ..source_range.start + opening + 2 + target.trim_end().len();
            let label = inner
                .split_once('|')
                .map_or_else(|| target.trim().to_string(), |(_, label)| label.to_string());
            references.push(Reference {
                destination: destination.to_string(),
                destination_range: range,
                source_range,
                label,
                kind: ReferenceKind::Wiki,
                image,
                definition: false,
            });
            continue;
        }
        let Some(closing) = bracket_end(raw.as_bytes(), opening) else {
            continue;
        };
        let range =
            if link_type == LinkType::Inline && raw.as_bytes().get(closing + 1) == Some(&b'(') {
                destination_range(raw, closing + 2)
                    .map(|range| source_range.start + range.start..source_range.start + range.end)
            } else {
                parser
                    .reference_definitions()
                    .get(id.as_ref())
                    .and_then(|definition| definition_destination(content, &definition.span))
            };
        if let Some(range) = range {
            references.push(Reference {
                destination: destination.to_string(),
                destination_range: range,
                source_range,
                label: raw[opening + 1..closing].to_string(),
                kind: ReferenceKind::Markdown,
                image,
                definition: false,
            });
        }
    }
    references.sort_by_key(|reference| (reference.source_range.start, reference.definition));
    references
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destinations_preserve_source_ranges_labels_titles_aliases_and_unicode() {
        let markdown = "É 🙂 [**plano**](<folder/a b.md#seção> \"Título\")\r\n[[ folder/B#etapa |Meu rótulo]]\n[p](a\\(b\\).md#x \"Título\")\n![foto](../image.png)\n";
        let references = scan(markdown);
        assert_eq!(references.len(), 4);
        assert_eq!(
            &markdown[references[0].destination_range.clone()],
            "folder/a b.md#seção"
        );
        assert_eq!(references[0].label, "**plano**");
        assert_eq!(
            &markdown[references[1].destination_range.clone()],
            "folder/B#etapa"
        );
        assert_eq!(references[1].label, "Meu rótulo");
        assert_eq!(
            &markdown[references[2].destination_range.clone()],
            "a\\(b\\).md#x"
        );
        assert_eq!(references[2].destination, "a(b).md#x");
        assert!(references[3].image);
        let mut renamed = markdown.to_string();
        renamed.replace_range(
            references[0].destination_range.clone(),
            "moved/new.md#seção",
        );
        assert!(renamed.contains("[**plano**](<moved/new.md#seção> \"Título\")\r\n"));
    }

    #[test]
    fn reference_uses_share_the_definition_destination_without_rewriting_their_labels() {
        let markdown = "[Descrição][id]\n[id][] e [id]\n\n[id]: <../note.md#x> 'Título'\n";
        let references = scan(markdown);
        assert_eq!(references.len(), 4);
        assert_eq!(
            references
                .iter()
                .filter(|reference| reference.definition)
                .count(),
            1
        );
        assert!(references
            .iter()
            .all(|reference| &markdown[reference.destination_range.clone()] == "../note.md#x"));
        assert_eq!(
            references
                .iter()
                .filter(|reference| !reference.definition)
                .map(|reference| reference.label.as_str())
                .collect::<Vec<_>>(),
            vec!["Descrição", "id", "id"]
        );
    }

    #[test]
    fn code_fences_inline_code_escaped_literals_and_nested_labels_are_respected() {
        let markdown = "`[code](old.md)` e `[[old]]`\n\\[escaped](old.md)\n\n~~~~\n```\n[code](old.md)\n[[old]]\n~~~~\n\n    [indented](old.md)\n\n[um [rótulo] e `]`](note.md)\n";
        let references = scan(markdown);
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].destination, "note.md");
        assert_eq!(references[0].label, "um [rótulo] e `]`");
    }

    #[test]
    fn embedded_wiki_destinations_and_unicode_reference_labels_have_exact_ranges() {
        let content =
            "![[folder/note#section|rótulo]]\n[texto][STRASSE]\n\n[straße]: <folder/note.md>\n";
        let refs = scan(content);
        assert_eq!(refs.len(), 3);
        assert!(refs[0].image);
        assert_eq!(
            &content[refs[0].destination_range.clone()],
            "folder/note#section"
        );
        assert_eq!(
            &content[refs[1].destination_range.clone()],
            "folder/note.md"
        );
        assert_eq!(refs[1].label, "texto");
    }
}
