use crate::{
    Document, EvidenceChunk, OpenApiOperation, OpenApiSchema, Requirement, Result, SearchFilter,
};
use async_trait::async_trait;

#[async_trait]
pub trait CatalogRepository: Send + Sync {
    async fn documents(&self) -> Result<Vec<Document>>;
    async fn document(&self, source: &str) -> Result<Document>;
    async fn chunks(&self, source: &str, locator: &str) -> Result<Vec<EvidenceChunk>>;
    async fn chunk(&self, id: &str) -> Result<EvidenceChunk>;
    async fn operations(
        &self,
        version: &str,
        path: &str,
        method: &str,
    ) -> Result<Vec<OpenApiOperation>>;
    async fn schema(&self, source: &str, name: &str) -> Result<OpenApiSchema>;
    async fn schemas(&self, version: &str, name: &str) -> Result<Vec<OpenApiSchema>>;
    async fn requirements(&self, version: Option<&str>) -> Result<Vec<Requirement>>;
    async fn requirement(&self, id: &str) -> Result<Requirement>;
    async fn health(&self) -> Result<serde_json::Value>;
}

#[async_trait]
pub trait SearchRepository: Send + Sync {
    async fn search(
        &self,
        query: &str,
        filter: SearchFilter,
        limit: usize,
    ) -> Result<Vec<(String, f32)>>;
}
