use async_trait::async_trait;
use bg_spec_core::*;
use std::{path::Path, sync::Arc};
use tantivy::{
    Index, IndexReader, ReloadPolicy, TantivyDocument, Term,
    collector::{
        DocSetCollector, TopDocs,
        sort_key::{SortBySimilarityScore, SortByString},
    },
    query::{AllQuery, BooleanQuery, Occur, Query, TermQuery},
    schema::{FAST, Field, IndexRecordOption, STORED, STRING, Schema, TEXT, Value},
    tokenizer::TokenStream,
};

fn storage(error: impl std::fmt::Display) -> Error {
    Error::Storage(error.to_string())
}

#[derive(Clone)]
struct Fields {
    id: Field,
    source: Field,
    version: Field,
    kind: Field,
    locator: Field,
    page: Field,
    title: Field,
    content: Field,
}

fn schema() -> (Schema, Fields) {
    let mut builder = Schema::builder();
    let fields = Fields {
        id: builder.add_text_field("chunk_id", STRING | STORED | FAST),
        source: builder.add_text_field("source_id", STRING),
        version: builder.add_text_field("version", STRING),
        kind: builder.add_text_field("kind", STRING),
        locator: builder.add_text_field("locator", STRING),
        page: builder.add_u64_field("page", tantivy::schema::INDEXED),
        title: builder.add_text_field("title", TEXT),
        content: builder.add_text_field("content", TEXT),
    };
    (builder.build(), fields)
}

#[derive(Clone)]
pub struct TantivySearch {
    inner: Arc<SearchInner>,
}

struct SearchInner {
    index: Index,
    reader: IndexReader,
    fields: Fields,
}

impl TantivySearch {
    pub fn build(path: &Path, chunks: &[EvidenceChunk], generation: &str) -> Result<()> {
        std::fs::create_dir_all(path)?;
        let (schema, fields) = schema();
        let index = Index::create_in_dir(path, schema).map_err(storage)?;
        let mut writer = index
            .writer_with_num_threads::<TantivyDocument>(1, 32_000_000)
            .map_err(storage)?;
        for chunk in chunks.iter().filter(|c| c.searchable) {
            let mut document = TantivyDocument::default();
            document.add_text(fields.id, &chunk.chunk_id);
            document.add_text(fields.source, &chunk.provenance.source_id);
            document.add_text(fields.version, &chunk.provenance.version.0);
            document.add_text(fields.kind, chunk.provenance.kind.as_str());
            document.add_text(fields.locator, &chunk.provenance.locator.0);
            document.add_u64(fields.page, u64::from(chunk.page.unwrap_or(0)));
            document.add_text(fields.title, &chunk.title);
            document.add_text(fields.content, &chunk.text);
            writer.add_document(document).map_err(storage)?;
        }
        let mut commit = writer.prepare_commit().map_err(storage)?;
        commit.set_payload(generation);
        commit.commit().map_err(storage)?;
        writer.wait_merging_threads().map_err(storage)?;
        Ok(())
    }

    pub fn open(path: &Path, generation: &str) -> Result<Self> {
        let index = Index::open_in_dir(path).map_err(storage)?;
        let (expected, fields) = schema();
        if index.schema() != expected
            || index.load_metas().map_err(storage)?.payload.as_deref() != Some(generation)
        {
            return Err(Error::Integrity(
                "search schema/generation mismatch; re-index".into(),
            ));
        }
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()
            .map_err(storage)?;
        Ok(Self {
            inner: Arc::new(SearchInner {
                index,
                reader,
                fields,
            }),
        })
    }

    pub fn count(&self) -> u64 {
        self.inner.reader.searcher().num_docs()
    }

    pub fn chunk_ids(&self) -> Result<Vec<String>> {
        let searcher = self.inner.reader.searcher();
        let addresses = searcher
            .search(&AllQuery, &DocSetCollector)
            .map_err(storage)?;
        let mut ids = Vec::with_capacity(addresses.len());
        for address in addresses {
            let document: TantivyDocument = searcher.doc(address).map_err(storage)?;
            let id = document
                .get_first(self.inner.fields.id)
                .and_then(|value| value.as_str())
                .ok_or_else(|| Error::Integrity("search document missing chunk_id".into()))?;
            ids.push(id.to_owned());
        }
        ids.sort();
        Ok(ids)
    }

    fn search_sync(
        &self,
        query: &str,
        filter: SearchFilter,
        limit: usize,
    ) -> Result<Vec<(String, f32)>> {
        if !(1..=100).contains(&limit) {
            return Err(Error::Invalid("limit must be 1..100".into()));
        }
        let fields = &self.inner.fields;
        let mut tokenizer = self
            .inner
            .index
            .tokenizer_for_field(fields.content)
            .map_err(storage)?;
        let mut stream = tokenizer.token_stream(query);
        let mut terms = std::collections::BTreeSet::new();
        while let Some(token) = stream.next() {
            terms.insert(token.text.clone());
            if terms.len() > 128 {
                return Err(Error::Invalid(
                    "query exceeds 128 distinct search tokens".into(),
                ));
            }
        }
        if terms.is_empty() {
            return Err(Error::Invalid("query contains no searchable tokens".into()));
        }
        // Literal token search: caller text cannot inject query-parser field selectors or filters.
        let mut text_queries: Vec<(Occur, Box<dyn Query>)> = Vec::new();
        for term in terms {
            for field in [fields.content, fields.title] {
                text_queries.push((
                    Occur::Should,
                    Box::new(TermQuery::new(
                        Term::from_field_text(field, &term),
                        IndexRecordOption::WithFreqs,
                    )),
                ));
            }
        }
        let mut queries: Vec<(Occur, Box<dyn Query>)> =
            vec![(Occur::Must, Box::new(BooleanQuery::new(text_queries)))];
        for (field, value) in [
            (fields.version, filter.version),
            (fields.kind, filter.kind.map(|k| k.as_str().into())),
            (fields.source, filter.source),
        ] {
            if let Some(value) = value {
                queries.push((
                    Occur::Must,
                    Box::new(TermQuery::new(
                        Term::from_field_text(field, &value),
                        IndexRecordOption::Basic,
                    )),
                ));
            }
        }
        let query = BooleanQuery::new(queries);
        let searcher = self.inner.reader.searcher();
        let collector = TopDocs::with_limit(limit).order_by((
            (SortBySimilarityScore, tantivy::Order::Desc),
            (SortByString::for_field("chunk_id"), tantivy::Order::Asc),
        ));
        let hits = searcher.search(&query, &collector).map_err(storage)?;
        let mut results = Vec::new();
        for ((score, _), address) in hits {
            let document: TantivyDocument = searcher.doc(address).map_err(storage)?;
            let id = document
                .get_first(fields.id)
                .and_then(|v| v.as_str())
                .ok_or_else(|| Error::Integrity("search document missing chunk_id".into()))?;
            results.push((id.to_owned(), score));
        }
        results.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        Ok(results)
    }
}

#[async_trait]
impl SearchRepository for TantivySearch {
    async fn search(
        &self,
        query: &str,
        filter: SearchFilter,
        limit: usize,
    ) -> Result<Vec<(String, f32)>> {
        let search = self.clone();
        let query = query.to_owned();
        tokio::task::spawn_blocking(move || search.search_sync(&query, filter, limit))
            .await
            .map_err(storage)?
    }
}
