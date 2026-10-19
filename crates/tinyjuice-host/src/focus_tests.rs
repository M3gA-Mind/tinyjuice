use super::*;
use serde_json::json;

#[test]
fn focus_is_removed_and_trimmed() {
    let mut arguments = json!({ "url": "u", "summary_focus": "  pricing  " });
    assert_eq!(
        take_summary_focus(&mut arguments).as_deref(),
        Some("pricing")
    );
    assert_eq!(arguments, json!({ "url": "u" }));
}

#[test]
fn blank_and_nonstring_focus_values_are_removed_without_a_value() {
    for value in [json!("  "), json!(4), json!(null)] {
        let mut arguments = json!({ "summary_focus": value });
        assert_eq!(take_summary_focus(&mut arguments), None);
        assert_eq!(arguments, json!({}));
    }
}

#[test]
fn only_the_bus_focus_property_is_recognized() {
    let property = summary_focus_property();
    assert!(declares_summary_focus(
        &json!({"properties":{"summary_focus":property}})
    ));
    assert!(!declares_summary_focus(
        &json!({"properties":{"summary_focus":{"type":"integer"}}})
    ));
}
