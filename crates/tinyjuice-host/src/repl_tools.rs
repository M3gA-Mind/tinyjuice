//! `tinytools::Tool` adapters that send REPL requests through the bus contract.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tinyjuice_bus::handle::{self, HandleArg};
use tinyjuice_bus::repl::{ReplLimits, ReplOp};
use tinyjuice_bus::tools::{ReplToolDeclaration, repl_tool_declarations};
use tinyjuice_bus::wire::{QueryError, QueryRequest, QueryResponse, QueryTarget};
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolResult, ToolRunContext};

/// The caller-owned source of module query results.
///
/// The run context is passed through exactly as supplied to the tool. A source
/// can use it to bind policy or runtime behavior to the invoking turn.
#[async_trait]
pub trait QuerySource: Send + Sync {
    /// Execute one bus query, preserving the original caller context.
    async fn query(
        &self,
        context: Option<&dyn ToolRunContext>,
        request: QueryRequest,
    ) -> Result<QueryResponse, String>;
}

/// Create tools from the current bus declarations.
pub fn repl_tools(
    source: Arc<dyn QuerySource>,
    limits: ReplLimits,
    artifacts_dir: Option<PathBuf>,
    max_artifact_bytes: u64,
    context_token: Option<String>,
) -> Vec<Box<dyn Tool>> {
    repl_tools_with_declarations(
        repl_tool_declarations(),
        source,
        limits,
        artifacts_dir,
        max_artifact_bytes,
        context_token,
    )
}

/// Create tools from frozen declarations recorded by the caller.
///
/// The supplied names, operations, descriptions and schemas are retained
/// verbatim, so a resumed session can execute its original tool surface while
/// using the caller's current source, limits and artifact policy.
pub fn repl_tools_with_declarations(
    declarations: Vec<ReplToolDeclaration>,
    source: Arc<dyn QuerySource>,
    limits: ReplLimits,
    artifacts_dir: Option<PathBuf>,
    max_artifact_bytes: u64,
    context_token: Option<String>,
) -> Vec<Box<dyn Tool>> {
    declarations
        .into_iter()
        .map(|declaration| {
            Box::new(ModuleReplTool {
                declaration,
                source: Arc::clone(&source),
                limits,
                artifacts_dir: artifacts_dir.clone(),
                max_artifact_bytes,
                context_token: context_token.clone(),
            }) as Box<dyn Tool>
        })
        .collect()
}

struct ModuleReplTool {
    declaration: ReplToolDeclaration,
    source: Arc<dyn QuerySource>,
    limits: ReplLimits,
    artifacts_dir: Option<PathBuf>,
    max_artifact_bytes: u64,
    context_token: Option<String>,
}

fn not_a_handle_message() -> &'static str {
    "juice: that is not a juice handle. A handle is the 32-character hex token named in a \
     stored-output footer (or its `⟦tj:<handle>⟧` marker); a tool call id such as `call_…` \
     is not one. For a `[tool_result_preview]` envelope, pass its `artifact_path` as the \
     handle, or read that file with file_read."
}

fn artifact_refused(reason: &str) -> String {
    format!(
        "juice: {reason}. Only an `artifact_path` under the workspace's \
         artifacts/tool-results directory can stand in for a handle; read other files with \
         file_read."
    )
}

/// Read a persisted tool-result artifact while enforcing its directory and size cap.
pub fn read_tool_result_artifact(dir: &Path, raw: &str, max_bytes: u64) -> Result<String, String> {
    const RELATIVE_PREFIX: &str = "artifacts/tool-results/";

    let raw = raw.trim();
    let path = Path::new(raw);
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(artifact_refused("that path contains `..`"));
    }
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        let forward = raw.replace('\\', "/");
        match forward
            .trim_start_matches("./")
            .strip_prefix(RELATIVE_PREFIX)
        {
            Some(rest) if !rest.is_empty() => dir.join(rest),
            _ => return Err(artifact_refused("that path is not a tool-result artifact")),
        }
    };
    let canonical_dir = std::fs::canonicalize(dir).ok();
    let lexically_inside = resolved.starts_with(dir)
        || canonical_dir
            .as_deref()
            .is_some_and(|canonical_dir| resolved.starts_with(canonical_dir));
    if !lexically_inside {
        return Err(artifact_refused("that path is not a tool-result artifact"));
    }
    let not_found = |_| artifact_refused("that artifact no longer exists");
    let canonical_dir = match canonical_dir {
        Some(canonical_dir) => canonical_dir,
        None => std::fs::canonicalize(dir).map_err(not_found)?,
    };
    let canonical = std::fs::canonicalize(&resolved).map_err(not_found)?;
    if !canonical.starts_with(&canonical_dir) {
        return Err(artifact_refused(
            "that path resolves outside the tool-results directory",
        ));
    }
    let metadata = std::fs::metadata(&canonical).map_err(not_found)?;
    if !metadata.is_file() {
        return Err(artifact_refused("that path is not a file"));
    }
    if metadata.len() > max_bytes {
        return Err(artifact_refused(&format!(
            "that artifact is {} bytes, over the {max_bytes}-byte limit",
            metadata.len()
        )));
    }
    let bytes = std::fs::read(&canonical).map_err(not_found)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn miss_message() -> &'static str {
    "juice: that handle is no longer stored (evicted, or from an earlier session). \
     Do NOT re-run the same tool call to regenerate it: the result would be stored \
     again under a new handle. Work from the preview already shown, or re-run with \
     narrower arguments so the result is small enough to keep in full."
}

impl ModuleReplTool {
    async fn query(
        &self,
        context: Option<&dyn ToolRunContext>,
        target: QueryTarget,
        mut args: Value,
    ) -> anyhow::Result<ToolResult> {
        if let Some(object) = args.as_object_mut() {
            object.insert("op".into(), Value::String(self.declaration.op.clone()));
        }
        let op: ReplOp = match serde_json::from_value(args) {
            Ok(op) => op,
            Err(error) => {
                return Ok(ToolResult::error(format!(
                    "juice: invalid arguments: {error}"
                )));
            }
        };
        let response = self
            .source
            .query(
                context,
                QueryRequest {
                    target,
                    op,
                    limits: self.limits,
                    context_token: self.context_token.clone(),
                    scope: None,
                },
            )
            .await;
        Ok(match response {
            Ok(Ok(output)) => ToolResult::json(serde_json::to_value(output)?),
            Ok(Err(QueryError::HandleNotFound)) => ToolResult::failed(miss_message()),
            Ok(Err(QueryError::InputTooLarge)) => {
                ToolResult::error("juice: input exceeds module limit")
            }
            Ok(Err(QueryError::InvalidPattern(pattern))) => {
                ToolResult::error(format!("juice: invalid pattern: {pattern}"))
            }
            Ok(Err(QueryError::EmptyQuery)) => ToolResult::error("juice: empty query"),
            Ok(Err(QueryError::Unsupported(operation))) => {
                ToolResult::error(format!("juice: {operation} support is not compiled in"))
            }
            Err(error) => ToolResult::error(format!("juice: {error}")),
        })
    }

    async fn query_artifact(
        &self,
        context: Option<&dyn ToolRunContext>,
        path: String,
        args: Value,
    ) -> anyhow::Result<ToolResult> {
        let Some(dir) = self.artifacts_dir.clone() else {
            return Ok(ToolResult::error(not_a_handle_message()));
        };
        let cap = self.max_artifact_bytes;
        let read = tokio::task::spawn_blocking(move || read_tool_result_artifact(&dir, &path, cap))
            .await
            .map_err(|error| anyhow::anyhow!("juice: artifact read task failed: {error}"))?;
        match read {
            Ok(content) => {
                self.query(context, QueryTarget::Content { content }, args)
                    .await
            }
            Err(message) => Ok(ToolResult::error(message)),
        }
    }
}

#[async_trait]
impl Tool for ModuleReplTool {
    fn name(&self) -> &str {
        &self.declaration.name
    }

    fn description(&self) -> &str {
        &self.declaration.description
    }

    fn parameters_schema(&self) -> Value {
        self.declaration.parameters.clone()
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, ToolCallOptions::default(), None)
            .await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        _options: ToolCallOptions,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let Some(raw) = args.get("handle").and_then(Value::as_str) else {
            return Ok(ToolResult::error("missing required argument: handle"));
        };
        match handle::classify(raw) {
            HandleArg::Handle(token) => {
                self.query(context, QueryTarget::Handle { token }, args)
                    .await
            }
            HandleArg::Path(path) => self.query_artifact(context, path, args).await,
            HandleArg::Invalid => Ok(ToolResult::error(not_a_handle_message())),
        }
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn is_concurrency_safe(&self, _args: &Value) -> bool {
        true
    }

    fn external_effect(&self) -> bool {
        false
    }

    fn max_result_size_chars(&self) -> Option<usize> {
        Some(self.limits.max_output_chars.saturating_mul(2))
    }
}

#[cfg(test)]
#[path = "repl_tools_tests.rs"]
mod tests;
