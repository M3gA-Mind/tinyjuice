use super::*;

#[test]
fn unavailable_notices_explain_the_reason_and_discourage_duplicate_tool_calls() {
    for (reason, explanation) in [
        (
            UnavailableReason::PayloadTooLarge,
            "exceeds the summarizer's size cap",
        ),
        (UnavailableReason::Disabled, "model summaries are disabled"),
        (UnavailableReason::Failed, "did not return a usable summary"),
        (UnavailableReason::TimedOut, "summarization timed out"),
    ] {
        let notice = reason.notice();
        assert!(notice.contains(explanation), "{notice}");
        assert!(
            notice.ends_with("Do not re-run the tool for a summary.]"),
            "{notice}"
        );
    }
}

#[test]
fn timeout_notice_makes_recovery_conditional_on_a_handle() {
    let notice = UnavailableReason::TimedOut.notice();
    assert!(notice.contains("If a recovery handle appears in its footer"));
    assert!(notice.contains("inspect the stored output with the juice_* tools"));
}
