use crate::chunking::{chunk_text, evidence};
use bg_spec_core::{
    Config, Document, Error, IndexedDocument, OpenApiOperation, OpenApiSchema, Result,
    SourceLocator,
    comparison::{pointer_escape, schema_references},
    sha256,
};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub fn parse_openapi(
    document: &Document,
    bytes: &[u8],
    config: &Config,
) -> Result<IndexedDocument> {
    let yaml =
        std::str::from_utf8(bytes).map_err(|e| Error::Invalid(format!("OpenAPI UTF-8: {e}")))?;
    let api: openapiv3::OpenAPI =
        serde_saphyr::from_str(yaml).map_err(|e| Error::Invalid(format!("OpenAPI: {e}")))?;
    if !api.openapi.starts_with("3.0.") {
        return Err(Error::Invalid("only OpenAPI 3.0.x is supported by openapiv3; convert 3.1/3.2 explicitly before ingestion".into()));
    }
    let root = serde_json::to_value(api)?;
    validate_references(&root, &root, 0)?;
    let mut indexed = IndexedDocument {
        document: Some(document.clone()),
        ..Default::default()
    };
    let paths = root["paths"]
        .as_object()
        .ok_or_else(|| Error::Invalid("OpenAPI paths must be an object".into()))?;
    add_unit(
        &mut indexed,
        document,
        &root["info"],
        "openapi:info",
        config,
    )?;
    for (path, item) in paths {
        if path.starts_with("x-") {
            continue;
        }
        let item = resolve(item, &root, &mut BTreeSet::new(), 0)?;
        let mut path_operations = BTreeMap::new();
        for method in [
            "get", "put", "post", "delete", "options", "head", "patch", "trace",
        ] {
            let Some(raw_operation) = item.get(method) else {
                continue;
            };
            let mut contract = resolve(raw_operation, &root, &mut BTreeSet::new(), 0)?;
            let object = contract
                .as_object_mut()
                .ok_or_else(|| Error::Invalid("operation must be an object".into()))?;
            let mut parameters = BTreeMap::new();
            for params in [item.get("parameters"), object.get("parameters")] {
                if let Some(Value::Array(params)) = params {
                    for parameter in params {
                        let parameter = resolve(parameter, &root, &mut BTreeSet::new(), 0)?;
                        let name = parameter["name"]
                            .as_str()
                            .ok_or_else(|| Error::Invalid("parameter missing name".into()))?;
                        let location = parameter["in"]
                            .as_str()
                            .ok_or_else(|| Error::Invalid("parameter missing in".into()))?;
                        parameters.insert((location.to_owned(), name.to_owned()), parameter);
                    }
                }
            }
            object.insert(
                "parameters".into(),
                Value::Array(parameters.into_values().collect()),
            );
            if !object.contains_key("security") {
                object.insert(
                    "security".into(),
                    root.get("security").cloned().unwrap_or_else(|| json!([])),
                );
            }
            if !object.contains_key("servers") {
                object.insert(
                    "servers".into(),
                    item.get("servers")
                        .or_else(|| root.get("servers"))
                        .cloned()
                        .unwrap_or_else(|| json!([])),
                );
            }
            object.insert("path".into(), json!(path));
            object.insert("method".into(), json!(method.to_ascii_uppercase()));
            // Include the definitions behind named security requirements.
            let names: BTreeSet<_> = object["security"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_object)
                .flat_map(|o| o.keys().cloned())
                .collect();
            let mut schemes = BTreeMap::new();
            for name in names {
                let scheme = root["components"]["securitySchemes"]
                    .get(&name)
                    .ok_or_else(|| Error::Invalid(format!("missing security scheme {name}")))?;
                schemes.insert(name, resolve(scheme, &root, &mut BTreeSet::new(), 0)?);
            }
            object.insert("securitySchemes".into(), serde_json::to_value(schemes)?);
            let locator = format!("operation:{}:{path}", method.to_ascii_uppercase());
            let serialized = serde_json::to_string(&contract)?;
            let mut provenance = document.provenance.clone();
            provenance.locator = SourceLocator(locator.clone());
            provenance.sha256 = sha256(serialized.as_bytes());
            indexed.operations.push(OpenApiOperation {
                provenance,
                path: path.clone(),
                method: method.to_ascii_uppercase(),
                operation_id: contract["operationId"].as_str().map(str::to_owned),
                summary: contract["summary"].as_str().map(str::to_owned),
                description: contract["description"].as_str().map(str::to_owned),
                parameters: contract["parameters"].clone(),
                request_body: contract["requestBody"].clone(),
                responses: contract["responses"].clone(),
                security: contract["security"].clone(),
                tags: contract.get("tags").cloned().unwrap_or_else(|| json!([])),
                referenced_schemas: schema_references(&contract),
                contract: contract.clone(),
            });
            add_unit(&mut indexed, document, &contract, &locator, config)?;
            path_operations.insert(method.to_ascii_uppercase(), contract);
        }
        add_unit(
            &mut indexed,
            document,
            &serde_json::to_value(path_operations)?,
            &format!("path:{path}"),
            config,
        )?;
    }
    if let Some(components) = root["components"].as_object() {
        for (category, entries) in components {
            let Some(entries) = entries.as_object() else {
                continue;
            };
            for (name, value) in entries {
                let value = resolve(value, &root, &mut BTreeSet::new(), 0)?;
                let locator = format!(
                    "json:#/components/{}/{}",
                    pointer_escape(category),
                    pointer_escape(name)
                );
                if category == "schemas" {
                    let mut provenance = document.provenance.clone();
                    provenance.locator = SourceLocator(locator.clone());
                    provenance.sha256 = sha256(serde_json::to_string(&value)?.as_bytes());
                    indexed.schemas.push(OpenApiSchema {
                        provenance,
                        name: name.clone(),
                        referenced_schemas: schema_references(&value),
                        schema: value.clone(),
                    });
                }
                add_unit(&mut indexed, document, &value, &locator, config)?;
            }
        }
    }
    Ok(indexed)
}

fn add_unit(
    indexed: &mut IndexedDocument,
    document: &Document,
    value: &Value,
    locator: &str,
    config: &Config,
) -> Result<()> {
    let text = serde_json::to_string(value)?;
    let searchable = text.chars().count() <= config.chunk_chars;
    indexed
        .chunks
        .push(evidence(document, &text, locator, None, searchable));
    if !searchable {
        indexed.chunks.extend(chunk_text(
            document,
            &text,
            locator,
            None,
            config.chunk_chars,
            config.chunk_overlap,
        )?);
    }
    Ok(())
}

fn validate_references(value: &Value, root: &Value, depth: usize) -> Result<()> {
    if depth > 128 {
        return Err(Error::Invalid("OpenAPI nesting exceeds 128".into()));
    }
    match value {
        Value::Object(object) => {
            if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
                let pointer = reference
                    .strip_prefix('#')
                    .filter(|s| s.starts_with('/'))
                    .ok_or_else(|| {
                        Error::Invalid(format!(
                            "external/non-pointer reference not allowed: {reference}"
                        ))
                    })?;
                if root.pointer(pointer).is_none() {
                    return Err(Error::Invalid(format!("unresolved reference: {reference}")));
                }
                if let Some(name) = reference.strip_prefix("#/components/schemas/")
                    && name.contains('/')
                {
                    return Err(Error::Invalid(
                        "nested schema references are unsupported; use a named component schema"
                            .into(),
                    ));
                }
            }
            for child in object.values() {
                validate_references(child, root, depth + 1)?;
            }
        }
        Value::Array(array) => {
            for child in array {
                validate_references(child, root, depth + 1)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn resolve(
    value: &Value,
    root: &Value,
    seen: &mut BTreeSet<String>,
    depth: usize,
) -> Result<Value> {
    if depth > 128 {
        return Err(Error::Invalid("reference expansion exceeds 128".into()));
    }
    match value {
        Value::Object(object) => {
            if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
                if reference.starts_with("#/components/schemas/") {
                    return Ok(value.clone());
                }
                if !seen.insert(reference.to_owned()) {
                    return Err(Error::Invalid(format!(
                        "non-schema reference cycle: {reference}"
                    )));
                }
                let target = reference
                    .strip_prefix('#')
                    .and_then(|p| root.pointer(p))
                    .ok_or_else(|| Error::Invalid(format!("unresolved reference: {reference}")))?;
                let resolved = resolve(target, root, seen, depth + 1)?;
                seen.remove(reference);
                return Ok(resolved);
            }
            object
                .iter()
                .map(|(key, child)| Ok((key.clone(), resolve(child, root, seen, depth + 1)?)))
                .collect::<Result<serde_json::Map<_, _>>>()
                .map(Value::Object)
        }
        Value::Array(values) => values
            .iter()
            .map(|v| resolve(v, root, seen, depth + 1))
            .collect::<Result<Vec<_>>>()
            .map(Value::Array),
        _ => Ok(value.clone()),
    }
}
