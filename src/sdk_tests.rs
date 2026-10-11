use super::*;
use serde_json::json;

#[tokio::test]
async fn sdk_compacts_command_output() {
    let mut lines = vec!["On branch main".to_string()];
    for index in 0..200 {
        lines.push(format!("\tmodified:   src/file_{index}.rs"));
    }
    let response = compress_request(SdkCompressionRequest {
        host: TinyJuiceHost::GenericJson,
        profile: AgentTokenjuiceCompression::Full,
        input: ToolExecutionInput {
            tool_name: "shell".to_string(),
            command: Some("git status".to_string()),
            combined_text: Some(lines.join("\n")),
            exit_code: Some(0),
            ..Default::default()
        },
        options: SdkCompressOptions {
            min_bytes_to_compress: Some(64),
            ..Default::default()
        },
    })
    .await;

    assert!(response.applied, "response: {response:?}");
    assert_eq!(response.host, TinyJuiceHost::GenericJson);
    assert_eq!(response.classification.content_kind, "plain_text");
    assert!(response.stats.reduced_chars < response.stats.raw_chars);
}

#[test]
fn parses_bare_tool_input_payload() {
    let request = request_from_json_value(json!({
        "toolName": "shell",
        "command": "cargo test",
        "combinedText": "ok"
    }))
    .expect("request");
    assert_eq!(request.input.tool_name, "shell");
    assert_eq!(request.input.command.as_deref(), Some("cargo test"));
}

#[test]
fn parses_codex_style_hook_payload() {
    let request = request_from_json_value(json!({
        "hook_event_name": "PostToolUse",
        "tool_name": "shell",
        "tool_input": {
            "command": "cargo test",
            "cwd": "/repo"
        },
        "tool_response": {
            "output": "test output",
            "exitCode": 0
        }
    }))
    .expect("request");
    assert_eq!(request.input.tool_name, "shell");
    assert_eq!(request.input.command.as_deref(), Some("cargo test"));
    assert_eq!(request.input.cwd.as_deref(), Some("/repo"));
    assert_eq!(request.input.combined_text.as_deref(), Some("test output"));
    assert_eq!(request.input.exit_code, Some(0));
}

#[test]
fn renders_hook_templates_as_json() {
    let codex = serde_json::from_str::<Value>(&host_template(TinyJuiceHost::Codex, "tinyjuice"))
        .expect("codex template");
    assert_eq!(
        codex["hooks"]["PostToolUse"][0]["matcher"].as_str(),
        Some("Bash")
    );
    assert!(
        codex["hooks"]["PostToolUse"][0]["hooks"][0]["command"]
            .as_str()
            .expect("command")
            .contains("codex-post-tool-use")
    );
    let codex_status = codex["hooks"]["PostToolUse"][0]["hooks"][0]["statusMessage"]
        .as_str()
        .expect("codex status");
    assert!(
        HOOK_STATUS_MESSAGES.contains(&codex_status),
        "unexpected status: {codex_status}"
    );

    let claude =
        serde_json::from_str::<Value>(&host_template(TinyJuiceHost::ClaudeCode, "tinyjuice"))
            .expect("claude template");
    assert!(
        claude["hooks"]["PostToolUse"][0]["hooks"][0]["command"]
            .as_str()
            .expect("command")
            .contains("claude-code-post-tool-use")
    );
    let claude_status = claude["hooks"]["PostToolUse"][0]["hooks"][0]["statusMessage"]
        .as_str()
        .expect("claude status");
    assert!(
        HOOK_STATUS_MESSAGES.contains(&claude_status),
        "unexpected status: {claude_status}"
    );
}

#[tokio::test]
async fn codex_hook_response_adds_context() {
    let mut lines = vec!["On branch main".to_string()];
    for index in 0..200 {
        lines.push(format!("\tmodified:   src/file_{index}.rs"));
    }
    let payload = json!({
        "hook_event_name": "PostToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": "git status"},
        "tool_response": {"output": lines.join("\n"), "exitCode": 0}
    });
    let response = compress_host_hook_payload(
        TinyJuiceHost::Codex,
        &payload.to_string(),
        SdkCompressOptions {
            min_bytes_to_compress: Some(64),
            ..Default::default()
        },
    )
    .await
    .expect("hook response")
    .expect("compressed");
    assert_eq!(
        response["hookSpecificOutput"]["hookEventName"].as_str(),
        Some("PostToolUse")
    );
    assert!(
        response["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .expect("context")
            .contains("M: src/file_")
    );
}

#[tokio::test]
async fn claude_hook_response_updates_tool_output() {
    let mut lines = vec!["On branch main".to_string()];
    for index in 0..200 {
        lines.push(format!("\tmodified:   src/file_{index}.rs"));
    }
    let payload = json!({
        "hook_event_name": "PostToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": "git status"},
        "tool_response": {"output": lines.join("\n"), "exitCode": 0}
    });
    let response = compress_host_hook_payload(
        TinyJuiceHost::ClaudeCode,
        &payload.to_string(),
        SdkCompressOptions {
            min_bytes_to_compress: Some(64),
            ..Default::default()
        },
    )
    .await
    .expect("hook response")
    .expect("compressed");
    assert!(
        response["hookSpecificOutput"]["updatedToolOutput"]
            .as_str()
            .expect("output")
            .contains("M: src/file_")
    );
}

#[tokio::test]
async fn hook_response_skips_non_matching_event() {
    let payload = json!({
        "hook_event_name": "PreToolUse",
        "tool_name": "Bash",
        "tool_input": {"command": "git status"}
    });
    let response = compress_host_hook_payload(
        TinyJuiceHost::Codex,
        &payload.to_string(),
        SdkCompressOptions::default(),
    )
    .await
    .expect("hook response");
    assert!(response.is_none());
}

#[test]
fn host_ids_are_parseable() {
    for spec in host_install_specs() {
        let serialized = serde_json::to_value(spec.host).expect("host serializes");
        assert_eq!(serialized, Value::String(spec.host.id().to_string()));
        let parsed = TinyJuiceHost::from_str(spec.host.id()).expect("host id");
        assert_eq!(parsed, spec.host);
    }
}

#[test]
fn sdk_options_roundtrip_explicit_policy_limits() {
    let opts = CompressOptions {
        router_enabled: false,
        ccr_enabled: false,
        search_enabled: false,
        code_enabled: false,
        html_enabled: false,
        min_bytes_to_compress: 17,
        min_bytes_to_compress_log: 23,
        ccr_min_tokens: 29,
        lossy_without_ccr: true,
        max_inline_chars: Some(31),
        code_target_ratio: Some(0.25),
        chars_per_token: 3.0,
        ..Default::default()
    };
    let converted = SdkCompressOptions::from(opts).into_compress_options();
    assert!(
        !converted.router_enabled
            && !converted.ccr_enabled
            && !converted.search_enabled
            && !converted.code_enabled
            && !converted.html_enabled
    );
    assert_eq!(converted.min_bytes_to_compress, 17);
    assert_eq!(converted.min_bytes_to_compress_log, 23);
    assert_eq!(converted.ccr_min_tokens, 29);
    assert!(converted.lossy_without_ccr);
    assert_eq!(converted.max_inline_chars, Some(31));
    assert_eq!(converted.code_target_ratio, Some(0.25));
    assert_eq!(converted.chars_per_token, 3.0);
}

#[tokio::test]
async fn configured_sdk_preserves_off_profile_and_reports_host() {
    let sdk = TinyJuiceSdk::new(TinyJuiceHost::RustHarness)
        .with_profile(AgentTokenjuiceCompression::Off)
        .with_options(CompressOptions {
            min_bytes_to_compress: 0,
            ..Default::default()
        });
    let response = sdk
        .compress_tool_output(ToolExecutionInput {
            tool_name: "shell".into(),
            stdout: Some("private payload".into()),
            ..Default::default()
        })
        .await;
    assert_eq!(response.inline_text, "private payload");
    assert_eq!(response.host, TinyJuiceHost::RustHarness);
    assert_eq!(response.profile, AgentTokenjuiceCompression::Off);
    assert!(!response.applied && !response.lossy);
    assert!(host_hook_response(&response).is_none());
}

#[test]
fn arguments_adapter_preserves_args_and_overrides_command_fields() {
    assert!(arguments_value(&ToolExecutionInput::default()).is_none());
    let input = ToolExecutionInput {
        args: Some(HashMap::from([
            ("path".into(), json!("file.rs")),
            ("command".into(), json!("old")),
        ])),
        command: Some("git status".into()),
        argv: Some(vec!["git".into(), "status".into()]),
        ..Default::default()
    };
    assert_eq!(
        arguments_value(&input),
        Some(json!({"path":"file.rs","command":"git status","argv":["git","status"]}))
    );
}

#[test]
fn host_aliases_and_templates_cover_generic_and_rust_harnesses() {
    for (name, host) in [
        ("open-human", TinyJuiceHost::OpenHuman),
        ("harness", TinyJuiceHost::RustHarness),
        ("json", TinyJuiceHost::GenericJson),
    ] {
        assert_eq!(TinyJuiceHost::from_str(name).unwrap(), host);
        assert!(!host.label().is_empty());
    }
    assert!(TinyJuiceHost::from_str("unsupported").is_err());
    for host in [TinyJuiceHost::OpenHuman, TinyJuiceHost::RustHarness] {
        let template = host_template(host, "tinyjuice");
        assert!(
            template.contains("TinyJuiceSdk::new")
                && template.contains(host.label().split_whitespace().next().unwrap_or(""))
        );
    }
    let template: Value =
        serde_json::from_str(&host_template(TinyJuiceHost::GenericJson, "tinyjuice")).unwrap();
    assert_eq!(template["host"], "generic-json");
}

#[test]
fn hook_field_adapters_filter_invalid_values_and_serialize_structured_output() {
    assert_eq!(
        array_string_field(&json!({"argv":[1,"git",false,"status"]}), &["argv"]),
        Some(vec!["git".into(), "status".into()])
    );
    assert_eq!(array_string_field(&json!({"argv":[1]}), &["argv"]), None);
    assert_eq!(
        i32_field(&json!({"exit_code":i64::MAX}), &["exit_code"]),
        None
    );
    assert_eq!(response_text(&Value::Null), None);
    assert_eq!(
        response_text(&json!({"nested":[1,2]})),
        Some(r#"{"nested":[1,2]}"#.into())
    );
}
