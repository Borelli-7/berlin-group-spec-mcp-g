use async_trait::async_trait;
use bg_spec_core::{
    CatalogRepository, Document, Error, EvidenceChunk, IndexedDocument, IndexedStatus,
    OpenApiOperation, OpenApiSchema, Provenance, Requirement, Result, sha256,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use std::path::Path;

pub const FORMAT_VERSION: &str = "1";
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

fn decode<T: DeserializeOwned>(rows: Vec<String>) -> Result<Vec<T>> {
    rows.into_iter()
        .map(|row| serde_json::from_str(&row).map_err(Error::from))
        .collect()
}

#[derive(Clone)]
pub struct SqliteCatalog {
    pool: SqlitePool,
}

impl SqliteCatalog {
    pub async fn open_read_only(path: &Path) -> Result<Self> {
        if !path.is_file() {
            return Err(Error::NotFound("catalog; run bg-spec index first".into()));
        }
        let options = SqliteConnectOptions::new()
            .filename(path)
            .read_only(true)
            .immutable(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .map_err(Error::storage)?;
        let result = Self { pool };
        let format: Option<String> =
            sqlx::query_scalar("SELECT value FROM metadata WHERE key='format_version'")
                .fetch_optional(&result.pool)
                .await
                .map_err(Error::storage)?;
        if format.as_deref() != Some(FORMAT_VERSION) {
            return Err(Error::Integrity(
                "unsupported catalog format; re-index".into(),
            ));
        }
        Ok(result)
    }

    pub async fn create(path: &Path, generation: &str) -> Result<Self> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Delete)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(Error::storage)?;
        MIGRATOR.run(&pool).await.map_err(Error::storage)?;
        for (key, value) in [
            ("format_version", FORMAT_VERSION),
            ("generation", generation),
        ] {
            sqlx::query("INSERT INTO metadata(key,value) VALUES (?,?)")
                .bind(key)
                .bind(value)
                .execute(&pool)
                .await
                .map_err(Error::storage)?;
        }
        Ok(Self { pool })
    }

    pub async fn write_document(&self, indexed: &IndexedDocument) -> Result<()> {
        let document = indexed
            .document
            .as_ref()
            .ok_or_else(|| Error::Integrity("missing document".into()))?;
        let mut tx = self.pool.begin().await.map_err(Error::storage)?;
        sqlx::query("INSERT INTO documents(source_id,version,kind,sha256,data) VALUES (?,?,?,?,?)")
            .bind(&document.provenance.source_id)
            .bind(&document.provenance.version.0)
            .bind(document.provenance.kind.as_str())
            .bind(&document.provenance.sha256)
            .bind(serde_json::to_string(document)?)
            .execute(&mut *tx)
            .await
            .map_err(Error::storage)?;
        for chunk in &indexed.chunks {
            sqlx::query(
                "INSERT INTO chunks(chunk_id,source_id,locator,searchable,data) VALUES (?,?,?,?,?)",
            )
            .bind(&chunk.chunk_id)
            .bind(&chunk.provenance.source_id)
            .bind(&chunk.provenance.locator.0)
            .bind(chunk.searchable)
            .bind(serde_json::to_string(chunk)?)
            .execute(&mut *tx)
            .await
            .map_err(Error::storage)?;
        }
        for operation in &indexed.operations {
            sqlx::query("INSERT INTO openapi_operations(source_id,version,path,method,data) VALUES (?,?,?,?,?)")
                .bind(&operation.provenance.source_id).bind(&operation.provenance.version.0).bind(&operation.path)
                .bind(&operation.method).bind(serde_json::to_string(operation)?).execute(&mut *tx).await.map_err(Error::storage)?;
        }
        for schema in &indexed.schemas {
            sqlx::query(
                "INSERT INTO openapi_schemas(source_id,version,name,data) VALUES (?,?,?,?)",
            )
            .bind(&schema.provenance.source_id)
            .bind(&schema.provenance.version.0)
            .bind(&schema.name)
            .bind(serde_json::to_string(schema)?)
            .execute(&mut *tx)
            .await
            .map_err(Error::storage)?;
        }
        tx.commit().await.map_err(Error::storage)?;
        Ok(())
    }

    pub async fn write_requirements(&self, requirements: &[Requirement]) -> Result<()> {
        let mut tx = self.pool.begin().await.map_err(Error::storage)?;
        for requirement in requirements {
            sqlx::query("INSERT INTO requirements(id,source_id,version,data) VALUES (?,?,?,?)")
                .bind(&requirement.mapping.id)
                .bind(&requirement.provenance.source_id)
                .bind(&requirement.mapping.version.0)
                .bind(serde_json::to_string(requirement)?)
                .execute(&mut *tx)
                .await
                .map_err(Error::storage)?;
            for source in &requirement.resolved_sources {
                sqlx::query("INSERT INTO requirement_sources(requirement_id,source_id,locator,data) VALUES (?,?,?,?)")
                    .bind(&requirement.mapping.id).bind(&source.source_id).bind(&source.locator.0)
                    .bind(serde_json::to_string(source)?).execute(&mut *tx).await.map_err(Error::storage)?;
            }
        }
        tx.commit().await.map_err(Error::storage)?;
        Ok(())
    }

    pub async fn export_document(&self, document: Document) -> Result<IndexedDocument> {
        let source = &document.provenance.source_id;
        let chunks = decode(
            sqlx::query_scalar("SELECT data FROM chunks WHERE source_id=? ORDER BY chunk_id")
                .bind(source)
                .fetch_all(&self.pool)
                .await
                .map_err(Error::storage)?,
        )?;
        let operations = decode(
            sqlx::query_scalar(
                "SELECT data FROM openapi_operations WHERE source_id=? ORDER BY path,method",
            )
            .bind(source)
            .fetch_all(&self.pool)
            .await
            .map_err(Error::storage)?,
        )?;
        let schemas = decode(
            sqlx::query_scalar("SELECT data FROM openapi_schemas WHERE source_id=? ORDER BY name")
                .bind(source)
                .fetch_all(&self.pool)
                .await
                .map_err(Error::storage)?,
        )?;
        Ok(IndexedDocument {
            document: Some(document),
            chunks,
            operations,
            schemas,
        })
    }

    pub async fn all_searchable_chunks(&self) -> Result<Vec<EvidenceChunk>> {
        decode(
            sqlx::query_scalar("SELECT data FROM chunks WHERE searchable=1 ORDER BY chunk_id")
                .fetch_all(&self.pool)
                .await
                .map_err(Error::storage)?,
        )
    }

    pub async fn generation(&self) -> Result<String> {
        sqlx::query_scalar("SELECT value FROM metadata WHERE key='generation'")
            .fetch_one(&self.pool)
            .await
            .map_err(Error::storage)
    }

    pub async fn doctor(&self) -> Result<Value> {
        let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_one(&self.pool)
            .await
            .map_err(Error::storage)?;
        let foreign_keys = sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&self.pool)
            .await
            .map_err(Error::storage)?;
        if integrity != "ok" || !foreign_keys.is_empty() {
            return Err(Error::Integrity("SQLite consistency check failed".into()));
        }
        for document in self.documents().await? {
            let indexed = self.export_document(document.clone()).await?;
            for chunk in &indexed.chunks {
                Self::check_provenance(&chunk.provenance, &document)?;
                Self::check_hash(chunk.text.as_bytes(), &chunk.provenance)?;
                if chunk.chunk_id
                    != sha256(
                        format!(
                            "{}\0{}\0{}",
                            chunk.provenance.document_id,
                            chunk.provenance.locator.0,
                            chunk.provenance.sha256
                        )
                        .as_bytes(),
                    )
                {
                    return Err(Error::Integrity(format!(
                        "chunk identity {}",
                        chunk.chunk_id
                    )));
                }
            }
            for operation in indexed.operations {
                Self::check_provenance(&operation.provenance, &document)?;
                Self::check_hash(
                    serde_json::to_string(&operation.contract)?.as_bytes(),
                    &operation.provenance,
                )?;
                for name in &operation.referenced_schemas {
                    self.schema(&operation.provenance.source_id, name).await?;
                }
            }
            for schema in indexed.schemas {
                Self::check_provenance(&schema.provenance, &document)?;
                Self::check_hash(
                    serde_json::to_string(&schema.schema)?.as_bytes(),
                    &schema.provenance,
                )?;
                for name in &schema.referenced_schemas {
                    self.schema(&schema.provenance.source_id, name).await?;
                }
            }
        }
        for requirement in self.requirements(None).await? {
            Self::check_provenance(
                &requirement.provenance,
                &self.document(&requirement.provenance.source_id).await?,
            )?;
            Self::check_hash(
                serde_json::to_string(&requirement.mapping)?.as_bytes(),
                &requirement.provenance,
            )?;
            for source in &requirement.resolved_sources {
                let chunks = self.chunks(&source.source_id, &source.locator.0).await?;
                if !chunks.iter().any(|c| &c.provenance == source) {
                    return Err(Error::Integrity(format!(
                        "requirement citation {}",
                        requirement.mapping.id
                    )));
                }
            }
        }
        self.health().await
    }

    fn check_provenance(provenance: &Provenance, document: &Document) -> Result<()> {
        if provenance.source_id != document.provenance.source_id
            || provenance.document_id != document.provenance.document_id
            || provenance.version != document.provenance.version
            || provenance.kind != document.provenance.kind
        {
            return Err(Error::Integrity(
                "record/document provenance mismatch".into(),
            ));
        }
        Ok(())
    }

    fn check_hash(content: &[u8], provenance: &Provenance) -> Result<()> {
        if sha256(content) != provenance.sha256 {
            return Err(Error::Integrity(format!(
                "content hash {} {}",
                provenance.source_id, provenance.locator.0
            )));
        }
        Ok(())
    }

    pub async fn close(&self) {
        self.pool.close().await;
    }

    pub async fn assert_read_only(&self) -> Result<()> {
        match sqlx::query("CREATE TABLE read_only_probe (id INTEGER)")
            .execute(&self.pool)
            .await
        {
            Err(sqlx::Error::Database(error)) if error.code().as_deref() == Some("8") => Ok(()),
            Err(error) => Err(Error::storage(error)),
            Ok(_) => Err(Error::Integrity("catalog is writable".into())),
        }
    }

    async fn one<T: DeserializeOwned>(&self, query: &'static str, value: &str) -> Result<T> {
        let data: Option<String> = sqlx::query_scalar(query)
            .bind(value)
            .fetch_optional(&self.pool)
            .await
            .map_err(Error::storage)?;
        serde_json::from_str(&data.ok_or_else(|| Error::NotFound(value.into()))?)
            .map_err(Error::from)
    }
}

#[async_trait]
impl CatalogRepository for SqliteCatalog {
    async fn documents(&self) -> Result<Vec<Document>> {
        decode(
            sqlx::query_scalar("SELECT data FROM documents ORDER BY source_id")
                .fetch_all(&self.pool)
                .await
                .map_err(Error::storage)?,
        )
    }
    async fn document(&self, source: &str) -> Result<Document> {
        self.one("SELECT data FROM documents WHERE source_id=?", source)
            .await
    }
    async fn chunks(&self, source: &str, locator: &str) -> Result<Vec<EvidenceChunk>> {
        decode(
            sqlx::query_scalar(
                "SELECT data FROM chunks WHERE source_id=? AND locator=? ORDER BY chunk_id",
            )
            .bind(source)
            .bind(locator)
            .fetch_all(&self.pool)
            .await
            .map_err(Error::storage)?,
        )
    }
    async fn chunk(&self, id: &str) -> Result<EvidenceChunk> {
        self.one("SELECT data FROM chunks WHERE chunk_id=?", id)
            .await
    }
    async fn operations(
        &self,
        version: &str,
        path: &str,
        method: &str,
    ) -> Result<Vec<OpenApiOperation>> {
        decode(sqlx::query_scalar("SELECT data FROM openapi_operations WHERE version=? AND path=? AND method=? ORDER BY source_id")
            .bind(version).bind(path).bind(method).fetch_all(&self.pool).await.map_err(Error::storage)?)
    }
    async fn schema(&self, source: &str, name: &str) -> Result<OpenApiSchema> {
        let rows = self.schemas_for_source(source, name).await?;
        rows.into_iter()
            .next()
            .ok_or_else(|| Error::NotFound(format!("{source} schema {name}")))
    }
    async fn schemas(&self, version: &str, name: &str) -> Result<Vec<OpenApiSchema>> {
        decode(
            sqlx::query_scalar(
                "SELECT data FROM openapi_schemas WHERE version=? AND name=? ORDER BY source_id",
            )
            .bind(version)
            .bind(name)
            .fetch_all(&self.pool)
            .await
            .map_err(Error::storage)?,
        )
    }
    async fn requirements(&self, version: Option<&str>) -> Result<Vec<Requirement>> {
        decode(
            sqlx::query_scalar(
                "SELECT data FROM requirements WHERE (? IS NULL OR version=?) ORDER BY id",
            )
            .bind(version)
            .bind(version)
            .fetch_all(&self.pool)
            .await
            .map_err(Error::storage)?,
        )
    }
    async fn requirement(&self, id: &str) -> Result<Requirement> {
        self.one("SELECT data FROM requirements WHERE id=?", id)
            .await
    }
    async fn health(&self) -> Result<Value> {
        #[derive(Serialize, sqlx::FromRow)]
        struct Counts {
            documents: i64,
            chunks: i64,
            operations: i64,
            schemas: i64,
            requirements: i64,
        }
        let counts: Counts = sqlx::query_as("SELECT (SELECT count(*) FROM documents) AS documents, (SELECT count(*) FROM chunks WHERE searchable=1) AS chunks, (SELECT count(*) FROM openapi_operations) AS operations, (SELECT count(*) FROM openapi_schemas) AS schemas, (SELECT count(*) FROM requirements) AS requirements")
            .fetch_one(&self.pool).await.map_err(Error::storage)?;
        let ocr: Vec<_> = self
            .documents()
            .await?
            .into_iter()
            .filter(|d| d.indexed_status == IndexedStatus::OcrRequired)
            .map(|d| json!({"source_id":d.provenance.source_id,"pages":d.ocr_required_pages}))
            .collect();
        Ok(
            json!({"status":if ocr.is_empty() {"ok"} else {"degraded"}, "generation":self.generation().await?, "counts":counts, "ocr_required":ocr}),
        )
    }
}

impl SqliteCatalog {
    async fn schemas_for_source(&self, source: &str, name: &str) -> Result<Vec<OpenApiSchema>> {
        decode(
            sqlx::query_scalar("SELECT data FROM openapi_schemas WHERE source_id=? AND name=?")
                .bind(source)
                .bind(name)
                .fetch_all(&self.pool)
                .await
                .map_err(Error::storage)?,
        )
    }
}
