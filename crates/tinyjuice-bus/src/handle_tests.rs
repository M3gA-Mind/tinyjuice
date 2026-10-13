use super::*;

#[test]
fn parse_only_accepts_alphanumeric_tokens() {
    assert_eq!(parse(" abc123 "), Some("abc123"));
    assert_eq!(parse("⟦tj:abc123⟧"), Some("abc123"));
    assert_eq!(parse("⟦tj: abc123 ⟧"), Some("abc123"));
    assert_eq!(parse("../x"), None);
    assert_eq!(parse(""), None);
    assert_eq!(parse("⟦tj:⟧"), None);
    assert_eq!(parse("call_abc"), None);
    assert_eq!(parse(&"f".repeat(MAX_HANDLE_LEN)), Some("f".repeat(MAX_HANDLE_LEN).as_str()));
    assert_eq!(parse(&"f".repeat(MAX_HANDLE_LEN + 1)), None);
}

#[test]
fn classify_separates_handles_paths_and_junk() {
    let hex = "0123456789abcdef0123456789abcdef";
    assert_eq!(classify(hex), HandleArg::Handle(hex.into()));
    assert_eq!(classify(&format!(" ⟦tj:{hex}⟧ ")), HandleArg::Handle(hex.into()));
    assert_eq!(
        classify("artifacts/tool-results/s/shell/call_1.txt"),
        HandleArg::Path("artifacts/tool-results/s/shell/call_1.txt".into())
    );
    assert_eq!(classify("a\\b"), HandleArg::Path("a\\b".into()));
    assert_eq!(classify("out.txt"), HandleArg::Path("out.txt".into()));
    assert_eq!(classify("call_abc123"), HandleArg::Invalid);
    assert_eq!(classify("not a handle"), HandleArg::Invalid);
    assert_eq!(classify(&"a".repeat(65)), HandleArg::Invalid);
}
