use super::*;
use crate::cache::MemoryCcrStore;
use serde_json::json;

#[tokio::test]
async fn adapters_validate_arguments_and_query_the_bound_store() {
    let store = Arc::new(MemoryCcrStore::new(4, 1024));
    let handle = store.put("alpha\nbeta needle\n").token().to_string();
    let tools = repl_tools(store, ReplLimits::default());
    let find = tools
        .iter()
        .find(|tool| tool.name() == "juice_find")
        .unwrap();
    assert!(!find.description().is_empty());
    assert!(find.parameters_schema().is_object());
    assert_eq!(find.permission_level(), PermissionLevel::ReadOnly);
    assert!(find.is_concurrency_safe(&json!({})));
    assert!(!find.external_effect());
    assert_eq!(
        find.max_result_size_chars(),
        Some(ReplLimits::default().max_output_chars * 2)
    );
    for args in [json!({}), json!(null), json!({"handle": handle})] {
        assert!(find.execute(args).await.unwrap().is_error);
    }
    let result = find
        .execute(json!({"handle": handle, "query": "needle"}))
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.output().contains("beta needle"));
    assert!(
        find.execute(json!({"handle": "missing", "query": "needle"}))
            .await
            .unwrap()
            .is_error
    );
    assert!(
        find.execute(json!({"handle": handle, "query": "[", "mode": "regex"}))
            .await
            .unwrap()
            .is_error
    );
}
