use super::*;

#[test]
fn repl_tool_constants_match_declarations() {
    let names: Vec<String> = repl_tool_declarations()
        .into_iter()
        .map(|d| d.name)
        .collect();
    assert_eq!(names, REPL_TOOLS);
    assert_eq!(RETRIEVE, "juice_retrieve");
    assert_eq!(LEGACY_RETRIEVE, "tinyjuice_retrieve");
}
