use super::*;

fn input(transcript: Vec<SubagentEvent>, max_bytes: usize) -> SubagentSummaryInput {
    SubagentSummaryInput {
        task: "inspect auth flow".to_string(),
        transcript,
        evidence_policy: EvidencePolicy::default(),
        max_bytes,
    }
}

#[test]
fn long_transcript_reduces_to_conclusion_plus_evidence() {
    let transcript = vec![
        SubagentEvent::user("Find the auth handler."),
        SubagentEvent::tool("grep", "src/auth.rs:42: pub fn login() {}\nnoise"),
        SubagentEvent::assistant(
            "Findings:\n- login is defined in src/auth.rs:42\n\nConclusion: edit auth.rs",
        ),
    ];
    let output = summarize_subagent_transcript(&input(transcript, 4_000));

    assert!(output.conclusion.contains("login is defined"));
    assert_eq!(output.evidence.len(), 1);
    assert_eq!(output.evidence[0].path.as_deref(), Some("src/auth.rs"));
    assert_eq!(output.evidence[0].line_start, Some(42));
    assert_eq!(output.findings[0].evidence_indices, vec![0]);
}

#[test]
fn failed_exploration_keeps_failure_reason() {
    let mut failed = SubagentEvent::tool("shell", "error: permission denied");
    failed.metadata.exit_code = Some(1);
    failed.metadata.failed = true;
    let output = summarize_subagent_transcript(&input(vec![failed], 4_000));

    assert!(output.findings[0].text.contains("exit code 1"));
    assert!(output.evidence[0].snippet.contains("permission denied"));
    assert!(output.open_questions[0].contains("error"));
}

#[test]
fn contradictory_findings_remain_uncertainty() {
    let transcript = vec![
        SubagentEvent::assistant("src/a.rs:10 says enabled"),
        SubagentEvent::assistant("contradiction: src/b.rs:20 says disabled"),
    ];
    let output = summarize_subagent_transcript(&input(transcript, 4_000));

    assert!(
        output
            .open_questions
            .iter()
            .any(|question| question.contains("contradiction"))
    );
    assert_eq!(output.evidence.len(), 2);
}

#[test]
fn byte_budget_produces_omission_report() {
    let transcript = vec![
        SubagentEvent::tool("grep", "src/a.rs:10: first\nsrc/b.rs:20: second"),
        SubagentEvent::assistant("Conclusion: src/a.rs:10 and src/b.rs:20 need review"),
    ];
    let output = summarize_subagent_transcript(&input(transcript, 120));
    let rendered = format_subagent_summary_markdown(&output);

    assert!(rendered.len() <= 120);
    assert!(output.omitted.byte_truncated);
    assert!(output.omitted.omitted_evidence > 0 || output.omitted.omitted_findings > 0);
}

#[test]
fn markdown_renders_inferences_ranges_tools_and_all_omission_counts() {
    let output = SubagentSummaryOutput {
        conclusion: String::new(),
        findings: vec![Finding {
            text: "inferred finding".into(),
            evidence_indices: vec![0, 1],
            inferred: true,
        }],
        evidence: vec![
            EvidenceRef {
                path: Some("src/a.rs".into()),
                line_start: Some(2),
                line_end: Some(4),
                tool_name: None,
                snippet: "range evidence".into(),
            },
            EvidenceRef {
                path: None,
                line_start: None,
                line_end: None,
                tool_name: Some("shell".into()),
                snippet: "tool evidence".into(),
            },
        ],
        open_questions: vec!["still unknown".into()],
        omitted: OmissionReport {
            omitted_events: 2,
            omitted_findings: 3,
            omitted_evidence: 4,
            omitted_open_questions: 5,
            byte_truncated: true,
        },
    };
    let rendered = format_subagent_summary_markdown(&output);
    for expected in [
        "No final conclusion",
        "[evidence: #1, #2]",
        "[inference]",
        "src/a.rs:2-4",
        "shell — tool evidence",
        "2 transcript event(s)",
        "3 finding(s)",
        "4 evidence item(s)",
        "5 open question(s)",
        "byte budget truncation",
    ] {
        assert!(rendered.contains(expected), "{rendered}");
    }
}

#[test]
fn bounds_truncate_each_collection_and_zero_budget_clears_content() {
    let mut output = SubagentSummaryOutput {
        conclusion: "conclusion".into(),
        findings: vec![
            Finding {
                text: "finding".into(),
                evidence_indices: vec![],
                inferred: false
            };
            3
        ],
        evidence: vec![
            EvidenceRef {
                path: None,
                line_start: None,
                line_end: None,
                tool_name: None,
                snippet: "evidence".into()
            };
            3
        ],
        open_questions: vec!["question".into(); 3],
        omitted: OmissionReport::default(),
    };
    let policy = EvidencePolicy {
        max_findings: 1,
        max_evidence: 1,
        max_open_questions: 1,
        ..Default::default()
    };
    apply_policy_bounds(&mut output, &input(vec![], 4000), policy);
    assert_eq!(output.findings.len(), 1);
    assert_eq!(output.evidence.len(), 1);
    assert_eq!(output.open_questions.len(), 1);
    assert_eq!(output.omitted.omitted_findings, 2);
    assert_eq!(output.omitted.omitted_evidence, 2);
    assert_eq!(output.omitted.omitted_open_questions, 2);
    enforce_byte_budget(&mut output, 0);
    assert!(
        output.conclusion.is_empty()
            && output.findings.is_empty()
            && output.evidence.is_empty()
            && output.open_questions.is_empty()
    );
    assert!(output.omitted.byte_truncated);
}

#[test]
fn evidence_metadata_handles_blank_snippets_and_shrinks_unicode() {
    let mut event = SubagentEvent::tool("read", "\nlong ééééé evidence");
    event.metadata.path = Some("src/lib.rs".into());
    event.metadata.line_start = Some(1);
    let mut request = input(vec![event.clone(), event], 4000);
    request.evidence_policy.max_snippet_chars = 8;
    let out = summarize_subagent_transcript(&request);
    assert_eq!(out.evidence.len(), 1);
    assert_eq!(out.evidence[0].snippet, "long éé...");
    let mut failed = SubagentEvent::tool("shell", "error without conclusion");
    failed.metadata.failed = true;
    let findings = fallback_findings(
        &input(vec![failed.clone(), failed], 4000),
        EvidencePolicy {
            max_findings: 1,
            ..Default::default()
        },
    );
    assert_eq!(findings.len(), 1);
    assert!(findings[0].text.contains("shell:"));
}
