use crate::chunking::{chunk_text, evidence};
use bg_spec_core::*;

pub struct PdfPage {
    pub number: u32,
    pub text: String,
}

pub trait PdfExtractor: Send + Sync {
    fn extract(&self, bytes: &[u8]) -> Result<Vec<PdfPage>>;
}

pub struct PdfOxideExtractor;

impl PdfExtractor for PdfOxideExtractor {
    fn extract(&self, bytes: &[u8]) -> Result<Vec<PdfPage>> {
        let pdf = pdf_oxide::PdfDocument::from_bytes(bytes.to_vec())
            .map_err(|e| Error::Extraction(format!("PDF open: {e}")))?;
        let count = pdf
            .page_count()
            .map_err(|e| Error::Extraction(format!("PDF page count: {e}")))?;
        if count == 0 || count > 10_000 {
            return Err(Error::Extraction("PDF must contain 1..10000 pages".into()));
        }
        let mut pages = Vec::with_capacity(count);
        let mut total = 0;
        for index in 0..count {
            let text = pdf
                .extract_text(index)
                .map_err(|e| Error::Extraction(format!("PDF page {}: {e}", index + 1)))?;
            total += text.len();
            if text.len() > 2_000_000 || total > 64_000_000 {
                return Err(Error::Extraction(
                    "extracted PDF text exceeds page/document limits".into(),
                ));
            }
            pages.push(PdfPage {
                number: (index + 1) as u32,
                text,
            });
        }
        Ok(pages)
    }
}

pub fn parse_pdf(
    mut document: Document,
    bytes: &[u8],
    extractor: &dyn PdfExtractor,
    config: &Config,
) -> Result<IndexedDocument> {
    let mut chunks = Vec::new();
    for page in extractor.extract(bytes)? {
        if page
            .text
            .chars()
            .filter(|c| c.is_alphanumeric())
            .take(3)
            .count()
            < 3
        {
            document.indexed_status = IndexedStatus::OcrRequired;
            document.ocr_required_pages.push(page.number);
            tracing::warn!(source_id = %document.provenance.source_id, page = page.number, "ocr_required: no usable extracted text");
            continue;
        }
        let locator = format!("page:{}", page.number);
        chunks.push(evidence(
            &document,
            &page.text,
            &locator,
            Some(page.number),
            false,
        ));
        chunks.extend(chunk_text(
            &document,
            &page.text,
            &locator,
            Some(page.number),
            config.chunk_chars,
            config.chunk_overlap,
        )?);
    }
    Ok(IndexedDocument {
        document: Some(document),
        chunks,
        ..Default::default()
    })
}
