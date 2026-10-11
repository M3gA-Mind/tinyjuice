use super::*;
use crate::conversation::{ToolCall, ToolResultMessage};

#[test]
fn summary_message_has_metadata_tag_and_end_marker() {
    let summary = StructuredSummary {
        goal: vec!["finish P2-3".to_owned()],
        ..StructuredSummary::default()
    };
    let message = compaction_summary_message(&summary);

    assert!(is_compaction_summary_message(&message));
    let ConversationMessage::Chat(chat) = message else {
        panic!("expected chat summary");
    };
    assert!(chat.content.contains(SUMMARY_END_MARKER));
    assert_eq!(chat.role, "system");
    assert_eq!(
        chat.metadata
            .as_ref()
            .and_then(|metadata| metadata.get("tinyjuice"))
            .and_then(|metadata| metadata.get("kind"))
            .and_then(Value::as_str),
        Some(SUMMARY_METADATA_KIND)
    );
}

#[test]
fn repeated_compaction_updates_existing_summary_without_nesting() {
    let first = StructuredSummary {
        goal: vec!["old".to_owned()],
        ..StructuredSummary::default()
    };
    let second = StructuredSummary {
        goal: vec!["new".to_owned()],
        ..StructuredSummary::default()
    };
    let messages = vec![
        ConversationMessage::system("system"),
        compaction_summary_message(&first),
        ConversationMessage::user("latest"),
    ];

    let (updated, report) = upsert_compaction_summary(&messages, &second);

    assert!(report.previous_summary_updated);
    assert_eq!(report.removed_existing_summaries, 1);
    assert_eq!(
        updated
            .iter()
            .filter(|message| is_compaction_summary_message(message))
            .count(),
        1
    );
    let rehydrated =
        rehydrate_summary_from_message(&updated[1]).expect("summary metadata should rehydrate");
    assert_eq!(rehydrated.goal, vec!["new"]);
}

#[test]
fn auth_and_network_failures_preserve_messages() {
    for kind in [SummaryErrorKind::Auth, SummaryErrorKind::Network] {
        let decision = decide_summary_failure(
            &SummaryError::new(kind, "failed"),
            SummaryFailurePolicy {
                deterministic_fallback_enabled: true,
                allow_lossy_fallback: true,
            },
        );

        assert_eq!(decision.action, SummaryFailureAction::PreserveMessages);
        assert!(!decision.messages_dropped);
    }
}

#[test]
fn deterministic_fallback_extracts_local_anchors_and_redacts() {
    let messages = vec![
        ConversationMessage::user("please inspect /tmp/app/src/main.rs token=abc"),
        ConversationMessage::AssistantToolCalls {
            text: None,
            tool_calls: vec![ToolCall::new(
                "call-1",
                "read_file",
                r#"{"path":"/tmp/app/Cargo.toml"}"#,
            )],
            metadata: None,
        },
        ConversationMessage::tool_results(vec![ToolResultMessage::new(
            "call-1",
            "error: failed to read /tmp/app/Cargo.toml password=hunter2",
        )]),
    ];

    let summary = deterministic_fallback_summary(&messages, DeterministicSummaryOptions::default());

    assert!(summary.locally_generated);
    assert!(summary.lower_confidence);
    assert!(
        summary
            .critical_context
            .iter()
            .any(|item| item.contains("may be incomplete"))
    );
    assert!(
        summary
            .relevant_files
            .contains(&"/tmp/app/Cargo.toml".to_owned())
    );
    let rendered = render_structured_summary(&summary);
    assert!(!rendered.contains("hunter2"));
    assert!(!rendered.contains("token=abc"));
}

#[test]
fn normalizes_historical_summary_wrappers() {
    let rendered = render_structured_summary(&StructuredSummary {
        critical_context: vec!["keep this".to_owned()],
        ..StructuredSummary::default()
    });

    let normalized = normalize_summary_text(&rendered);

    assert!(!normalized.contains(SUMMARY_END_MARKER));
    assert!(normalized.contains("keep this"));
}

#[test]
fn prompt_carries_previous_summary_and_historical_tool_context_without_nesting() {
    let previous = StructuredSummary {
        goal: vec!["previous goal".into()],
        ..Default::default()
    };
    let mut request = SummaryRequest::new(vec![
        ConversationMessage::user("historical ask"),
        compaction_summary_message(&previous),
        ConversationMessage::AssistantToolCalls {
            text: Some("reading files".into()),
            tool_calls: vec![ToolCall::new("call", "read_file", "{}")],
            metadata: None,
        },
        ConversationMessage::tool_results(vec![ToolResultMessage::new("call", "file contents")]),
    ]);
    assert_eq!(request.max_output_bytes, 12 * 1024);
    request.previous_summary = Some(previous);
    let prompt = build_summary_prompt(&request);
    assert!(prompt.contains("Previous structured summary"));
    assert!(prompt.contains("previous goal"));
    assert!(prompt.contains("chat role=user content=historical ask"));
    assert!(prompt.contains("calls=[call:read_file]"));
    assert!(prompt.contains("tool_results [call:file contents]"));
    assert!(!prompt.contains(SUMMARY_END_MARKER));
}

#[test]
fn failure_policy_requires_explicit_lossy_fallback_and_retries_ineffective_compression() {
    let default = SummaryFailurePolicy::default();
    for kind in [
        SummaryErrorKind::NoProvider,
        SummaryErrorKind::ModelUnavailable,
        SummaryErrorKind::Timeout,
        SummaryErrorKind::MalformedOutput,
        SummaryErrorKind::Other,
        SummaryErrorKind::Permission,
    ] {
        let error = SummaryError::new(kind, "unavailable");
        assert_eq!(
            decide_summary_failure(&error, default).action,
            SummaryFailureAction::PreserveMessages
        );
        let allowed = SummaryFailurePolicy {
            allow_lossy_fallback: true,
            ..default
        };
        let decision = decide_summary_failure(&error, allowed);
        let fallback = !matches!(kind, SummaryErrorKind::Other | SummaryErrorKind::Permission);
        assert_eq!(decision.messages_dropped, fallback);
        assert_eq!(
            decision.action,
            if fallback {
                SummaryFailureAction::UseDeterministicFallback
            } else {
                SummaryFailureAction::PreserveMessages
            }
        );
    }
    assert_eq!(
        decide_summary_failure(
            &SummaryError::new(SummaryErrorKind::IneffectiveCompression, ""),
            default
        )
        .action,
        SummaryFailureAction::RetryLater
    );
}

#[test]
fn fallback_collects_nested_paths_actions_and_blockers_with_section_limits() {
    let messages = vec![
        compaction_summary_message(&StructuredSummary::default()),
        ConversationMessage::assistant("finished /tmp/result password=hidden"),
        ConversationMessage::system("blocked by denied access"),
        ConversationMessage::AssistantToolCalls {
            text: Some("reading ./src/lib.rs".into()),
            tool_calls: vec![ToolCall::new(
                "read",
                "read_file",
                r#"{"nested":[{"path":"/tmp/input","workdir":"../work","ignored":42}]}"#,
            )],
            metadata: None,
        },
        ConversationMessage::user("a".repeat(500)),
        ConversationMessage::user("second ask should not fit"),
    ];
    let summary = deterministic_fallback_summary(
        &messages,
        DeterministicSummaryOptions {
            max_items_per_section: 1,
            ..Default::default()
        },
    );
    assert_eq!(summary.completed_actions.len(), 1);
    assert!(!summary.completed_actions[0].contains("hidden"));
    assert_eq!(summary.blockers, vec!["blocked by denied access"]);
    assert_eq!(summary.active_state, vec!["tool call read: read_file"]);
    assert_eq!(summary.relevant_files.len(), 1);
    assert_eq!(summary.historical_pending_asks.len(), 1);
    assert!(summary.historical_pending_asks[0].ends_with("..."));
    let mut paths = BTreeSet::new();
    collect_json_paths(
        &json!({"nested":[{"path":"/tmp/input","workdir":"../work"}]}),
        &mut paths,
    );
    assert!(paths.contains("/tmp/input") && paths.contains("../work"));
}

#[test]
fn summary_budget_clamps_sections_and_handles_below_wrapper_budget() {
    let mut summary = StructuredSummary {
        goal: vec!["é".repeat(500); 20],
        ..Default::default()
    };
    clamp_summary(&mut summary, 1200);
    assert!(render_structured_summary(&summary).len() <= 1200);
    assert!(summary.goal.len() < 20);
    clamp_summary(&mut summary, 0);
    assert!(summary.goal.is_empty());
    assert!(rehydrate_summary_from_message(&ConversationMessage::tool_results(vec![])).is_none());
    let normalized =
        normalize_summary_text(&render_structured_summary(&StructuredSummary::default()));
    assert!(!normalized.contains(SUMMARY_PREFIX));
    assert!(!normalized.contains(SUMMARY_END_MARKER));
    let (out, report) = upsert_compaction_summary(
        &[
            ConversationMessage::system("system"),
            ConversationMessage::tool_results(vec![]),
            ConversationMessage::user("live"),
        ],
        &StructuredSummary::default(),
    );
    assert_eq!(report.inserted_index, 1);
    assert_eq!(out.len(), 4);
}
