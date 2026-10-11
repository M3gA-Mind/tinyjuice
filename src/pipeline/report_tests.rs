use super::*;

#[test]
fn skip_reason_labels_are_stable_for_host_reports() {
    for (reason, label) in [
        (PipelineSkipReason::RouterDisabled, "router_disabled"),
        (PipelineSkipReason::BelowMinBytes, "below_min_bytes"),
        (PipelineSkipReason::NoCompressor, "no_compressor"),
        (PipelineSkipReason::NoSavings, "no_savings"),
        (PipelineSkipReason::LowBloat, "low_bloat"),
        (PipelineSkipReason::CcrDisabled, "ccr_disabled"),
        (PipelineSkipReason::CcrNotRetained, "ccr_not_retained"),
        (PipelineSkipReason::FooterWouldGrow, "footer_would_grow"),
        (PipelineSkipReason::RecoveryTool, "recovery_tool"),
        (PipelineSkipReason::Other("host_policy"), "host_policy"),
    ] {
        assert_eq!(reason.as_str(), label);
    }
}

#[test]
fn applied_report_preserves_recovery_and_bloat_metadata() {
    let bloat = BloatEstimate {
        score: 50,
        reason: crate::pipeline::BloatReason::LowSignal,
    };
    let report = PipelineReport::applied(
        ContentKind::Log,
        100,
        40,
        CompressorKind::Generic,
        true,
        Some("token".into()),
    )
    .with_bloat_estimate(bloat);
    assert_eq!(report.original_bytes, 100);
    assert_eq!(report.compacted_bytes, 40);
    assert_eq!(report.ccr_tokens, ["token"]);
    assert!(report.lossy);
    assert_eq!(report.bloat_estimate, Some(bloat));
    assert_eq!(
        report.applied_steps[0].compressor,
        Some(CompressorKind::Generic)
    );
    assert!(report.skip_reason.is_none());
}
