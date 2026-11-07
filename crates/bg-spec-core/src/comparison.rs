use crate::{ChangeKind, CompatibilityChange, Provenance};
use serde_json::Value;
use std::collections::BTreeSet;

pub fn structural_diff(
    before: Option<&Value>,
    after: Option<&Value>,
    location: &str,
    sources: &[Provenance],
) -> Vec<CompatibilityChange> {
    let mut changes = Vec::new();
    diff(before, after, location, sources, &mut changes);
    if changes.is_empty() {
        changes.push(CompatibilityChange {
            kind: ChangeKind::Unchanged,
            location: location.into(),
            fact: "structurally unchanged".into(),
            before: None,
            after: None,
            sources: sources.to_vec(),
        });
    }
    changes
}

fn diff(
    before: Option<&Value>,
    after: Option<&Value>,
    location: &str,
    sources: &[Provenance],
    changes: &mut Vec<CompatibilityChange>,
) {
    if before == after {
        return;
    }
    if let (Some(Value::Object(a)), Some(Value::Object(b))) = (before, after) {
        for key in a.keys().chain(b.keys()).collect::<BTreeSet<_>>() {
            diff(
                a.get(key),
                b.get(key),
                &format!("{location}/{}", pointer_escape(key)),
                sources,
                changes,
            );
        }
        return;
    }
    let kind = match (before, after) {
        (None, Some(_)) => ChangeKind::Added,
        (Some(_), None) => ChangeKind::Removed,
        _ => ChangeKind::Modified,
    };
    let field = location.rsplit('/').next().unwrap_or("");
    let fact = match field {
        "type" => "property type changed",
        "required" => "required changed",
        "enum" => "enum changed",
        "format" => "format changed",
        "nullable" => "nullable changed",
        _ if location.contains("/properties/") => match kind {
            ChangeKind::Added => "property added",
            ChangeKind::Removed => "property removed",
            _ => "property modified",
        },
        _ => match kind {
            ChangeKind::Added => "field added",
            ChangeKind::Removed => "field removed",
            _ => "field modified",
        },
    };
    changes.push(CompatibilityChange {
        kind,
        location: location.into(),
        fact: fact.into(),
        before: before.cloned(),
        after: after.cloned(),
        sources: sources.to_vec(),
    });
}

pub fn pointer_escape(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

pub fn schema_references(value: &Value) -> Vec<String> {
    fn walk(value: &Value, found: &mut BTreeSet<String>) {
        match value {
            Value::Object(map) => {
                if let Some(Value::String(reference)) = map.get("$ref")
                    && let Some(name) = reference.strip_prefix("#/components/schemas/")
                {
                    found.insert(name.replace("~1", "/").replace("~0", "~"));
                }
                for child in map.values() {
                    walk(child, found);
                }
            }
            Value::Array(values) => {
                for child in values {
                    walk(child, found);
                }
            }
            _ => {}
        }
    }
    let mut found = BTreeSet::new();
    walk(value, &mut found);
    found.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sha256;
    use serde_json::json;

    #[test]
    fn schema_changes_are_facts_not_verdicts() {
        let a = json!({"properties":{"id":{"type":"string","nullable":true,"enum":["a"],"format":"uuid"}},"required":["id"]});
        let b = json!({"properties":{"id":{"type":"integer","nullable":false,"enum":["b"],"format":"int64"},"name":{"type":"string"}},"required":[]});
        let diff = structural_diff(Some(&a), Some(&b), "schema", &[]);
        for expected in [
            "property type changed",
            "nullable changed",
            "enum changed",
            "format changed",
            "required changed",
            "property added",
        ] {
            assert!(diff.iter().any(|d| d.fact == expected), "{expected}");
        }
    }

    #[test]
    fn operation_add_remove_and_response_change() {
        let a = json!({"responses":{"200":{"description":"ok"}}});
        let b = json!({"responses":{"201":{"description":"created"}}});
        let changes = structural_diff(Some(&a), Some(&b), "operation", &[]);
        assert_eq!(changes.len(), 2);
        assert_eq!(
            structural_diff(Some(&a), Some(&a), "", &[])[0].kind,
            ChangeKind::Unchanged
        );
        assert_eq!(
            structural_diff(None, Some(&a), "", &[])[0].kind,
            ChangeKind::Added
        );
    }

    #[test]
    fn known_sha256() {
        assert_eq!(
            sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
