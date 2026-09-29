use bg_spec_core::Config;
use bg_spec_mcp::server::SpecMcp;
use bg_spec_store::Snapshot;
use rmcp::{ServiceExt, model::CallToolRequestParams};
use serde_json::{Value, json};
use std::path::Path;

fn inventory(root: &Path) -> std::collections::BTreeMap<std::path::PathBuf, String> {
    fn visit(root: &Path, result: &mut std::collections::BTreeMap<std::path::PathBuf, String>) {
        for entry in std::fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, result);
            } else {
                result.insert(
                    path.clone(),
                    bg_spec_core::sha256(&std::fs::read(path).unwrap()),
                );
            }
        }
    }
    let mut result = std::collections::BTreeMap::new();
    visit(root, &mut result);
    result
}

#[tokio::test]
async fn initialize_list_schemas_invocations_errors_and_read_only() {
    let temp = tempfile::tempdir().unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut config = Config::load(&root.join("config/config.toml")).unwrap();
    config.data_dir = temp.path().join("data");
    bg_spec_indexer::index(&config).await.unwrap();
    let before_data = inventory(&config.data_dir);
    let before_corpus = inventory(&config.corpus_dir);
    let snapshot = Snapshot::open(&config).await.unwrap();
    snapshot.catalog.assert_read_only().await.unwrap();
    let pointer = std::fs::read(config.data_dir.join("CURRENT")).unwrap();
    let (client_io, server_io) = tokio::io::duplex(1024 * 1024);
    let mcp = SpecMcp::new(&snapshot, &config);
    let server = tokio::spawn(async move {
        let server = mcp.serve(server_io).await.unwrap();
        server.waiting().await.unwrap();
    });
    let client = ().serve(client_io).await.unwrap();
    assert!(client.peer_info().unwrap().capabilities.tools.is_some());
    let tools = client.list_all_tools().await.unwrap();
    let mut names: Vec<_> = tools.iter().map(|t| t.name.to_string()).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "compare_v1_v2",
            "find_requirement",
            "get_endpoint_requirements",
            "health",
            "list_sources",
            "read_openapi_endpoint",
            "read_openapi_schema",
            "read_source",
            "search_specification",
            "trace_requirement"
        ]
    );
    for tool in &tools {
        assert_eq!(
            tool.annotations.as_ref().unwrap().read_only_hint,
            Some(true)
        );
        assert_eq!(
            tool.annotations.as_ref().unwrap().destructive_hint,
            Some(false)
        );
        assert_eq!(tool.input_schema["type"], "object");
        assert_eq!(tool.input_schema["additionalProperties"], false);
    }
    let endpoint = json!({"version":"openfinance-v2","path":"/accounts/{accountId}/transactions","method":"GET"});
    let calls = [
        ("get_endpoint_requirements", endpoint.clone()),
        ("read_openapi_endpoint", endpoint),
        (
            "trace_requirement",
            json!({"requirement_id":"FIXTURE-OFV2-TRANSACTIONS-001"}),
        ),
        (
            "read_source",
            json!({"source_id":"fixture-v2-guidelines","locator":"page:1"}),
        ),
        (
            "compare_v1_v2",
            json!({"path":"/accounts/{accountId}/transactions","method":"GET"}),
        ),
        (
            "read_openapi_schema",
            json!({"version":"openfinance-v2","name":"Transaction"}),
        ),
        (
            "search_specification",
            json!({"query":"transactions","version":"openfinance-v2","kind":"pdf","limit":10}),
        ),
        (
            "find_requirement",
            json!({"query":"FIXTURE-OFV2-TRANSACTIONS-001"}),
        ),
        ("list_sources", json!({})),
        ("health", json!({})),
    ];
    for (name, arguments) in calls {
        let reply = client
            .call_tool(
                CallToolRequestParams::new(name)
                    .with_arguments(arguments.as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        assert_ne!(reply.is_error, Some(true), "{name}: {reply:?}");
        assert!(reply.structured_content.is_some());
        assert!(!reply.content.is_empty());
    }
    for (name, arguments) in [
        (
            "read_source",
            json!({"source_id":"../../etc/passwd","locator":"page:1"}),
        ),
        (
            "search_specification",
            json!({"query":"transactions","limit":0}),
        ),
        (
            "search_specification",
            json!({"query":"transactions","kind":"shell"}),
        ),
        (
            "read_openapi_endpoint",
            json!({"version":"openfinance-v2","path":"/missing","method":"GET"}),
        ),
    ] {
        let reply = client
            .call_tool(
                CallToolRequestParams::new(name)
                    .with_arguments(arguments.as_object().unwrap().clone()),
            )
            .await
            .unwrap();
        assert_eq!(reply.is_error, Some(true));
        let error: Value = reply.structured_content.unwrap();
        assert!(error["code"].is_string());
    }
    for (name, arguments) in [
        ("read_source", json!({"path":"/etc/passwd"})),
        ("health", json!({"write":true})),
        ("shell", json!({"command":"anything"})),
    ] {
        if let Ok(reply) = client
            .call_tool(
                CallToolRequestParams::new(name)
                    .with_arguments(arguments.as_object().unwrap().clone()),
            )
            .await
        {
            assert_eq!(
                reply.is_error,
                Some(true),
                "{name}: invalid invocation succeeded"
            );
        }
    }
    assert_eq!(
        pointer,
        std::fs::read(config.data_dir.join("CURRENT")).unwrap()
    );
    snapshot.catalog.assert_read_only().await.unwrap();
    client.cancel().await.unwrap();
    server.await.unwrap();
    assert_eq!(before_data, inventory(&config.data_dir));
    assert_eq!(before_corpus, inventory(&config.corpus_dir));
}
