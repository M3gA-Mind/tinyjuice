use super::*;

#[test]
fn projections_support_literals_field_counts_and_default_print() {
    let limits = ReplLimits::default();
    let (hits, _) = run(
        "alice 30\nbob 25",
        r#"{ print "name", $1, NF, 7 }"#,
        &limits,
    )
    .unwrap();
    assert_eq!(
        hits.iter().map(|hit| hit.text.as_str()).collect::<Vec<_>>(),
        ["name alice 2 7", "name bob 2 7"]
    );
    assert_eq!(
        run("alice 30", "{ print }", &limits).unwrap().0[0].text,
        "alice 30"
    );
    assert_eq!(
        run("alice\nbob", "!/alice/", &limits).unwrap().0[0].text,
        "bob"
    );
    for program in ["NR != 1", "NR < 2", "NR <= 1", "NR >= 2", r#"$1 < "bob""#] {
        assert_eq!(
            run("alice\nbob", program, &limits).unwrap().0.len(),
            1,
            "{program}"
        );
    }
    assert!(run("NaN", "$1 > 0", &limits).unwrap().0.is_empty());
}

#[test]
fn invalid_programs_are_rejected_and_output_is_bounded() {
    let limits = ReplLimits {
        max_lines: 1,
        max_line_chars: 3,
        ..Default::default()
    };
    for program in [
        "-F",
        "$1 ~ text",
        "bad ~ /x/",
        "bad == 1",
        "unrecognized",
        "{ print unknown }",
        "/[/",
        "",
    ] {
        assert!(run("alpha", program, &limits).is_err(), "{program}");
    }
    let (hits, dropped) = run("alpha\nbeta", "{ print }", &limits).unwrap();
    assert_eq!(hits[0].text, "alp…");
    assert_eq!(dropped, 1);
}
