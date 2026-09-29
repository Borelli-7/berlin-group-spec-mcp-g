CREATE TABLE metadata (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);
CREATE TABLE documents (
    source_id TEXT PRIMARY KEY NOT NULL,
    version TEXT NOT NULL,
    kind TEXT NOT NULL,
    sha256 TEXT NOT NULL,
    data TEXT NOT NULL CHECK(json_valid(data))
);
CREATE TABLE chunks (
    chunk_id TEXT PRIMARY KEY NOT NULL,
    source_id TEXT NOT NULL REFERENCES documents(source_id),
    locator TEXT NOT NULL,
    searchable INTEGER NOT NULL CHECK(searchable IN (0, 1)),
    data TEXT NOT NULL CHECK(json_valid(data))
);
CREATE INDEX chunks_source_locator ON chunks(source_id, locator);
CREATE TABLE openapi_operations (
    source_id TEXT NOT NULL REFERENCES documents(source_id),
    version TEXT NOT NULL,
    path TEXT NOT NULL,
    method TEXT NOT NULL,
    data TEXT NOT NULL CHECK(json_valid(data)),
    PRIMARY KEY(source_id, path, method)
);
CREATE INDEX operations_endpoint ON openapi_operations(version, path, method);
CREATE TABLE openapi_schemas (
    source_id TEXT NOT NULL REFERENCES documents(source_id),
    version TEXT NOT NULL,
    name TEXT NOT NULL,
    data TEXT NOT NULL CHECK(json_valid(data)),
    PRIMARY KEY(source_id, name)
);
CREATE INDEX schemas_name ON openapi_schemas(version, name);
CREATE TABLE requirements (
    id TEXT PRIMARY KEY NOT NULL,
    source_id TEXT NOT NULL REFERENCES documents(source_id),
    version TEXT NOT NULL,
    data TEXT NOT NULL CHECK(json_valid(data))
);
CREATE INDEX requirements_version ON requirements(version);
CREATE TABLE requirement_sources (
    requirement_id TEXT NOT NULL REFERENCES requirements(id),
    source_id TEXT NOT NULL REFERENCES documents(source_id),
    locator TEXT NOT NULL,
    data TEXT NOT NULL CHECK(json_valid(data)),
    PRIMARY KEY(requirement_id, source_id, locator)
);
