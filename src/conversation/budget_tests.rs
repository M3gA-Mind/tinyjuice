use super::*;
use crate::conversation::{ChatMessage, ToolResultMessage};

#[test]
fn threshold_stays_below_small_effective_window() {
    let budget = ConversationBudget {
        context_length: 64_000,
        max_output_tokens: 8_000,
        threshold_ratio: 0.9,
        minimum_context_floor: 64_000,
    };
    let effective = effective_input_window(budget.context_length, budget.max_output_tokens);
    assert!(threshold_tokens(budget) < effective);
}

#[test]
fn output_reservation_reduces_input_budget() {
    assert_eq!(effective_input_window(128_000, 16_000), 112_000);
}

#[test]
fn huge_old_tool_result_does_not_force_preserving_recent_history() {
    let messages = vec![
        ConversationMessage::Chat(ChatMessage::system("sys")),
        ConversationMessage::tool_results(vec![ToolResultMessage::new("old", "x".repeat(20_000))]),
        ConversationMessage::user("latest ask"),
        ConversationMessage::assistant("latest answer"),
    ];

    let start = select_tail_by_budget(&messages, 1, 20, 2, 1.5);
    assert_eq!(start, 2);
}

#[test]
fn short_transcript_selects_whole_region() {
    let messages = vec![
        ConversationMessage::system("sys"),
        ConversationMessage::user("hi"),
        ConversationMessage::assistant("hello"),
    ];
    assert_eq!(select_tail_by_budget(&messages, 1, 100, 2, 1.25), 1);
}

#[test]
fn protected_head_decays_after_first_compaction_but_keeps_system() {
    let messages = vec![
        ConversationMessage::system("sys"),
        ConversationMessage::user("opening task"),
        ConversationMessage::assistant("ack"),
        ConversationMessage::user("latest"),
    ];

    let first = protected_head_end(
        &messages,
        HeadProtection {
            protect_first_n_messages: 3,
            compaction_count: 0,
            ..Default::default()
        },
    );
    let repeated = protected_head_end(
        &messages,
        HeadProtection {
            protect_first_n_messages: 3,
            compaction_count: 1,
            ..Default::default()
        },
    );

    assert_eq!(first, 3);
    assert_eq!(repeated, 1);
}

#[test]
fn empty_windows_and_fully_protected_transcripts_select_nothing() {
    assert_eq!(
        threshold_tokens(ConversationBudget {
            context_length: 10,
            max_output_tokens: 20,
            ..Default::default()
        }),
        0
    );
    let messages = vec![ConversationMessage::user("hello")];
    assert_eq!(
        select_tail_by_budget_detail(&messages, usize::MAX, 100, 1, 1.0),
        TailBudgetSelection {
            tail_start: 1,
            selected_tokens: 0,
            message_count: 0
        }
    );
    assert_eq!(
        protected_head_end(
            &messages,
            HeadProtection {
                protect_system_prompt: false,
                protect_first_n_messages: 1,
                decay_head_after_first_compaction: false,
                compaction_count: 2,
            }
        ),
        1
    );
}

#[test]
fn assistant_tool_estimates_include_arguments_and_optional_text() {
    use crate::conversation::ToolCall;
    let call = ToolCall::new("id", "search", "query");
    let without_text = ConversationMessage::assistant_tool_calls(vec![call.clone()]);
    let with_text = ConversationMessage::AssistantToolCalls {
        text: Some("searching".into()),
        tool_calls: vec![call],
        metadata: None,
    };
    assert_eq!(
        estimate_message_tokens(&without_text),
        4 + 12
            + estimate_tokens("id") as usize
            + estimate_tokens("search") as usize
            + estimate_tokens("query") as usize
    );
    assert_eq!(
        estimate_message_tokens(&with_text),
        estimate_message_tokens(&without_text) + estimate_tokens("searching") as usize
    );
    assert_eq!(
        threshold_tokens(ConversationBudget {
            context_length: 100,
            max_output_tokens: 0,
            minimum_context_floor: 0,
            threshold_ratio: 0.5,
        }),
        50
    );
}
