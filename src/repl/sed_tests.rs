use super::*;

#[test]
fn patterns_support_case_insensitive_addresses_and_escaped_delimiters() {
    let limits = ReplLimits::default();
    assert_eq!(
        run("alpha\nBETA", "-n /beta/Ip", &limits).unwrap().0[0].text,
        "BETA"
    );
    assert_eq!(
        run("a/b", r"s/a\/b/replaced/", &limits).unwrap().0[0].text,
        "replaced"
    );
    assert_eq!(
        run("ALPHA", "s/alpha/beta/i", &limits).unwrap().0[0].text,
        "beta"
    );
    assert_eq!(
        run("one\ntwo\nthree", "-n 2,1p", &limits).unwrap().0[0].text,
        "two"
    );
}

#[test]
fn malformed_sed_and_empty_scripts_return_errors() {
    let limits = ReplLimits::default();
    for program in [
        "",
        "/unterminated",
        "1,",
        "1",
        "s",
        "saxaxa",
        "s/a/b",
        "-n 1, p",
    ] {
        assert!(run("alpha", program, &limits).is_err(), "{program}");
    }
    let limits = ReplLimits {
        max_lines: 1,
        max_line_chars: 3,
        ..Default::default()
    };
    let (hits, dropped) = run("alpha\nbeta", "p", &limits).unwrap();
    assert_eq!(hits[0].text, "alp…");
    assert!(dropped > 0);
}
