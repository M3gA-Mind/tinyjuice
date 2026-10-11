use super::*;

#[test]
fn breakdown_separates_static_prefix_from_compressible_context() {
    let breakdown = ContextBreakdown::new(vec![
        ContextBucket::system_prompt(100),
        ContextBucket::tool_definitions(200),
        ContextBucket::conversation(700),
        ContextBucket::memory(50),
    ])
    .with_context_max_tokens(2_000);

    assert_eq!(breakdown.estimated_total_tokens, 1_050);
    assert_eq!(breakdown.static_prefix_tokens(), 300);
    assert_eq!(breakdown.compression_candidate_tokens(), 750);
    assert_eq!(breakdown.effective_prompt_tokens(), 1_050);
    assert_eq!(breakdown.usage_ratio(), Some(0.525));
}

#[test]
fn measured_prompt_tokens_override_rough_estimate() {
    let breakdown = ContextBreakdown::new(vec![
        ContextBucket::system_prompt(100),
        ContextBucket::conversation(400),
    ])
    .with_measured_prompt_tokens(800);

    assert_eq!(breakdown.estimated_total_tokens, 500);
    assert_eq!(breakdown.effective_prompt_tokens(), 800);
}

#[test]
fn bucket_measured_tokens_override_bucket_estimate() {
    let breakdown = ContextBreakdown::new(vec![
        ContextBucket::system_prompt(100),
        ContextBucket::conversation(400).with_measured_tokens(250),
    ]);

    assert_eq!(breakdown.effective_prompt_tokens(), 350);
    assert_eq!(breakdown.compression_candidate_tokens(), 250);
}

#[test]
fn serialized_breakdown_contains_no_raw_prompt_fields() {
    let breakdown = ContextBreakdown::new(vec![
        ContextBucket::estimated(ContextBucketKind::Skills, 42)
            .with_item_count(3)
            .with_byte_count(1_024),
    ]);
    let json = serde_json::to_string(&breakdown).unwrap();

    assert!(json.contains("skills"));
    assert!(!json.contains("content"));
    assert!(!json.contains("promptText"));
    assert!(!json.contains("raw"));
}

#[test]
fn bucket_labels_agree_with_the_wire_format() {
    for (kind, label) in [
        (ContextBucketKind::SystemPrompt, "system_prompt"),
        (ContextBucketKind::ToolDefinitions, "tool_definitions"),
        (ContextBucketKind::RulesContextFiles, "rules_context_files"),
        (ContextBucketKind::Skills, "skills"),
        (ContextBucketKind::McpTools, "mcp_tools"),
        (ContextBucketKind::Subagents, "subagents"),
        (ContextBucketKind::Memory, "memory"),
        (ContextBucketKind::Conversation, "conversation"),
        (ContextBucketKind::Other, "other"),
    ] {
        assert_eq!(kind.as_str(), label);
        assert_eq!(serde_json::to_value(kind).unwrap(), label);
    }
}

#[test]
fn empty_and_incremental_breakdowns_handle_missing_and_zero_windows() {
    let mut breakdown = ContextBreakdown::empty();
    assert_eq!(breakdown.effective_prompt_tokens(), 0);
    assert_eq!(breakdown.usage_ratio(), None);
    assert!(breakdown.bucket(ContextBucketKind::Memory).is_none());
    let bucket = ContextBucket::memory(20)
        .with_measured_tokens(10)
        .as_static_prefix();
    breakdown.add_bucket(bucket.clone());
    assert_eq!(breakdown.estimated_total_tokens, 20);
    assert_eq!(breakdown.static_prefix_tokens(), 10);
    assert_eq!(breakdown.compression_candidate_tokens(), 0);
    assert_eq!(breakdown.bucket(ContextBucketKind::Memory), Some(&bucket));
    assert_eq!(breakdown.with_context_max_tokens(0).usage_ratio(), None);
}
