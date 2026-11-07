use bg_spec_core::{Document, Error, EvidenceChunk, Result, SourceLocator, sha256};

pub fn chunk_text(
    document: &Document,
    text: &str,
    locator: &str,
    page: Option<u32>,
    max: usize,
    overlap: usize,
) -> Result<Vec<EvidenceChunk>> {
    if max == 0 || overlap >= max {
        return Err(Error::Invalid("invalid chunk size/overlap".into()));
    }
    let chars: Vec<char> = text.chars().collect();
    let mut headings = Vec::new();
    if page.is_some() {
        let mut offset = 0;
        for line in text.split_inclusive('\n') {
            if heading(line.trim()) {
                headings.push((offset, line.trim().to_owned()));
            }
            offset += line.chars().count();
        }
    }
    let mut result = Vec::new();
    let mut start = 0;
    while start < chars.len() {
        let section_end = headings
            .iter()
            .find(|(offset, _)| *offset > start)
            .map_or(chars.len(), |(offset, _)| *offset);
        let hard_end = (start + max).min(section_end);
        let mut end = hard_end;
        if hard_end < section_end {
            let lower = start + max / 2;
            let paragraph = (lower..hard_end)
                .rev()
                .find(|&i| i > start && chars[i - 1] == '\n' && chars[i] == '\n')
                .map(|i| i + 1);
            let sentence = (lower..hard_end)
                .rev()
                .find(|&i| i > start && chars[i].is_whitespace() && ".!?".contains(chars[i - 1]))
                .map(|i| i + 1);
            let word = (lower..hard_end)
                .rev()
                .find(|&i| chars[i].is_whitespace())
                .map(|i| i + 1);
            end = paragraph.or(sentence).or(word).unwrap_or(hard_end);
        }
        let body: String = chars[start..end].iter().collect();
        if !body.trim().is_empty() {
            let location = format!("{locator}:chars:{start}-{end}");
            let mut chunk = evidence(document, &body, &location, page, true);
            chunk.section = headings
                .iter()
                .rfind(|(offset, _)| *offset <= start)
                .map(|(_, title)| title.clone());
            result.push(chunk);
        }
        if end == chars.len() {
            break;
        }
        if end == section_end {
            start = end;
            continue;
        }
        let target = end.saturating_sub(overlap).max(start + 1);
        start = (target..end)
            .find(|&i| i == 0 || chars[i - 1].is_whitespace())
            .unwrap_or(end);
    }
    Ok(result)
}

fn heading(line: &str) -> bool {
    let line = line.trim();
    let numbered = line.chars().next().is_some_and(|c| c.is_ascii_digit())
        && line
            .split_whitespace()
            .next()
            .is_some_and(|s| s.contains('.'));
    !line.is_empty()
        && line.chars().count() <= 120
        && (numbered || (line.chars().any(char::is_alphabetic) && line == line.to_uppercase()))
}

pub fn evidence(
    document: &Document,
    text: &str,
    locator: &str,
    page: Option<u32>,
    searchable: bool,
) -> EvidenceChunk {
    let mut provenance = document.provenance.clone();
    provenance.locator = SourceLocator(locator.into());
    provenance.sha256 = sha256(text.as_bytes());
    EvidenceChunk {
        chunk_id: sha256(
            format!(
                "{}\0{}\0{}",
                provenance.document_id, locator, provenance.sha256
            )
            .as_bytes(),
        ),
        provenance,
        title: document.title.clone(),
        page,
        section: None,
        text: text.into(),
        searchable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bg_spec_core::{
        DocumentAuthority, DocumentKind, IndexedStatus, Provenance, SpecificationVersion,
    };
    fn document() -> Document {
        Document {
            provenance: Provenance {
                source_id: "test".into(),
                document_id: "test".into(),
                version: SpecificationVersion("test".into()),
                kind: DocumentKind::Pdf,
                locator: SourceLocator("document".into()),
                sha256: "test".into(),
            },
            title: "fixture".into(),
            authority: DocumentAuthority::Test,
            precedence: 0,
            path: "test.pdf".into(),
            indexed_status: IndexedStatus::Indexed,
            ocr_required_pages: vec![],
            manifest_sha256: "test".into(),
        }
    }
    #[test]
    fn boundaries_and_unicode_are_preserved() {
        let text = format!(
            "1. TRANSACTIONS\n\n{}\n\n{}\n\n{}",
            "The bank MUST return transactions. ".repeat(30),
            "Payment MUST include \u{20ac} currency. ".repeat(30),
            "End."
        );
        let chunks = chunk_text(&document(), &text, "page:1", Some(1), 600, 50).unwrap();
        assert!(chunks.len() > 2);
        for chunk in &chunks {
            assert!(chunk.text.chars().count() <= 600);
            assert!(text.contains(&chunk.text));
            assert_eq!(chunk.provenance.sha256, sha256(chunk.text.as_bytes()));
        }
        let chars: Vec<_> = text.chars().collect();
        let mut covered = vec![false; chars.len()];
        for chunk in chunks {
            let range = chunk
                .provenance
                .locator
                .0
                .split(":chars:")
                .nth(1)
                .unwrap()
                .to_owned();
            let (a, b) = range.split_once('-').unwrap();
            covered[a.parse::<usize>().unwrap()..b.parse::<usize>().unwrap()].fill(true);
        }
        assert!(covered.iter().all(|v| *v));
    }
    #[test]
    fn oversized_statement_and_empty_input() {
        assert_eq!(
            chunk_text(&document(), "", "page:1", Some(1), 10, 1)
                .unwrap()
                .len(),
            0
        );
        let chunks = chunk_text(&document(), &"x".repeat(100), "page:1", Some(1), 10, 1).unwrap();
        assert_eq!(chunks.len(), 10);
        assert!(chunk_text(&document(), "a", "", None, 10, 10).is_err());
    }

    #[test]
    fn headings_bound_sections_without_cross_section_overlap() {
        let text = "1. FIRST\nA complete statement.\n\n2. SECOND\nAnother statement.";
        let chunks = chunk_text(&document(), text, "page:1", Some(1), 6000, 500).unwrap();
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].section.as_deref(), Some("1. FIRST"));
        assert_eq!(chunks[1].section.as_deref(), Some("2. SECOND"));
        assert!(!chunks[0].text.contains("SECOND"));
        assert_eq!(
            chunks.iter().map(|c| c.text.as_str()).collect::<String>(),
            text
        );
    }
}
