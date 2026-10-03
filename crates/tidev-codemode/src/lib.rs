//! Code Mode execution backed by the in-process Monty runtime.

use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use monty::{MontyRun, RunProgress};
use monty_types::{
    CallArgs, CollectedStreams, CompileOptions, ExtFunctionResult, MontyException, MontyObject,
    ObjectRef, PrintStream, ResourceLimits, ResourceTracker,
};
use serde_json::{Map, Value};

/// Host-side settings for the in-process Monty runtime.
#[derive(Clone, Debug)]
pub struct CodeModeRuntimeConfig {
    pub max_feed_duration: Duration,
    pub max_suspensions: usize,
}

impl Default for CodeModeRuntimeConfig {
    fn default() -> Self {
        Self {
            max_feed_duration: Duration::from_secs(120),
            max_suspensions: 256,
        }
    }
}

/// A tool available as a host function inside Code Mode.
#[derive(Clone, Debug)]
pub struct CodeModeTool {
    pub name: String,
    pub description: String,
}

/// A single host function call made by a Code Mode script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeModeCall {
    pub name: String,
    pub succeeded: bool,
    pub duration_ms: u64,
}

/// Completed Code Mode output.
#[derive(Clone, Debug)]
pub struct CodeModeResult {
    pub output: String,
    pub calls: Vec<CodeModeCall>,
    pub failed: bool,
}

/// Callback used by the sandbox to invoke a tidev host tool.
#[async_trait]
pub trait CodeModeHost: Send + Sync {
    async fn call(&self, name: &str, arguments: Value) -> Result<Value>;
}

/// An in-process Monty runtime.
#[derive(Clone)]
pub struct CodeModeRuntime {
    config: CodeModeRuntimeConfig,
}

impl CodeModeRuntime {
    pub fn new(config: CodeModeRuntimeConfig) -> Self {
        Self { config }
    }

    /// Run one independent script session.
    pub async fn execute(
        &self,
        code: &str,
        tools: &[CodeModeTool],
        host: &dyn CodeModeHost,
    ) -> Result<CodeModeResult> {
        let input_names = tools
            .iter()
            .map(|tool| tool.name.clone())
            .collect::<Vec<_>>();
        let inputs = tools
            .iter()
            .map(|tool| MontyObject::function(tool.name.clone(), Some(tool.description.clone())))
            .collect::<Vec<_>>();

        let deadline = Instant::now() + self.config.max_feed_duration;
        let mut streams = CollectedStreams::default();
        let mut calls = Vec::new();
        let execution: Result<MontyObject> = async {
            let runner = MontyRun::new(
                code.to_owned(),
                "tidev-codemode.py",
                input_names,
                CompileOptions::default(),
            )
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            let limits = ResourceLimits::default()
                .max_feed_duration(self.config.max_feed_duration)
                .max_suspensions(self.config.max_suspensions);
            let mut progress = runner
                .start(
                    inputs,
                    ResourceTracker::new(limits),
                    collected_streams_writer(&mut streams),
                )
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            loop {
                match progress {
                    RunProgress::Complete(value) => return Ok(value),
                    RunProgress::FunctionCall(call) => {
                        let function_name = call.function_name.clone();
                        let arguments = call_arguments_to_json(&call.args);
                        let started_at = Instant::now();
                        let host_result = match arguments {
                            Ok(arguments) => {
                                let remaining = deadline.saturating_duration_since(Instant::now());
                                match tokio::time::timeout(
                                    remaining,
                                    host.call(&function_name, arguments),
                                )
                                .await
                                {
                                    Ok(result) => result,
                                    Err(_) => Err(anyhow::anyhow!("Code Mode request timed out")),
                                }
                            }
                            Err(error) => Err(error),
                        };
                        let succeeded = host_result.is_ok();
                        calls.push(CodeModeCall {
                            name: function_name,
                            succeeded,
                            duration_ms: started_at.elapsed().as_millis().max(1) as u64,
                        });
                        let result = match host_result {
                            Ok(value) => ExtFunctionResult::Return(json_to_monty(&value)?),
                            Err(error) => {
                                ExtFunctionResult::Error(MontyException::runtime_error(error))
                            }
                        };
                        progress = call
                            .resume(result, collected_streams_writer(&mut streams))
                            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
                    }
                    RunProgress::OsCall(call) => {
                        progress = call
                            .abort(
                                MontyException::runtime_error(
                                    "OS calls are unavailable in Code Mode",
                                ),
                                collected_streams_writer(&mut streams),
                            )
                            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
                    }
                    RunProgress::NameLookup(lookup) => {
                        progress = lookup
                            .resume(None::<MontyObject>, collected_streams_writer(&mut streams))
                            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
                    }
                    RunProgress::ResolveFutures(futures) => {
                        progress = futures
                            .abort(
                                MontyException::runtime_error(
                                    "async host functions are unavailable in Code Mode",
                                ),
                                collected_streams_writer(&mut streams),
                            )
                            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
                    }
                }
            }
        }
        .await;
        let (stdout, stderr) = collected_streams(&streams);

        match execution {
            Ok(value) => Ok(CodeModeResult {
                output: format_success(&stdout, &stderr, &value, &calls),
                calls,
                failed: false,
            }),
            Err(error) => Ok(CodeModeResult {
                output: format_failure(&stdout, &stderr, &error, &calls),
                calls,
                failed: true,
            }),
        }
    }
}

fn collected_streams_writer(streams: &mut CollectedStreams) -> monty_types::PrintWriter<'_> {
    monty_types::PrintWriter::collect_streams(streams)
}

fn collected_streams(streams: &CollectedStreams) -> (String, String) {
    let mut stdout = String::new();
    let mut stderr = String::new();
    for (stream, text) in streams.entries() {
        match stream {
            PrintStream::Stdout => stdout.push_str(text),
            PrintStream::Stderr => stderr.push_str(text),
        }
    }
    (stdout, stderr)
}

fn call_arguments_to_json(args: &CallArgs) -> Result<Value> {
    let positional = args.args().collect::<Vec<_>>();
    let kwargs = args.kwargs().collect::<Vec<_>>();
    if positional.len() == 1 && kwargs.is_empty() {
        let value = monty_to_json(positional[0]);
        if value.is_object() {
            return Ok(value);
        }
        return Ok(serde_json::json!({ "value": value }));
    }

    let mut object = Map::new();
    for (key, value) in kwargs {
        let key = key
            .as_str()
            .context("Code Mode keyword names must be strings")?;
        object.insert(key.to_string(), monty_to_json(value));
    }
    if !positional.is_empty() {
        object.insert(
            "args".to_string(),
            Value::Array(positional.into_iter().map(monty_to_json).collect()),
        );
    }
    Ok(Value::Object(object))
}

fn json_to_monty(value: &Value) -> Result<MontyObject> {
    Ok(match value {
        Value::Null => MontyObject::none(),
        Value::Bool(value) => MontyObject::bool(*value),
        Value::Number(value) => {
            if let Some(value) = value.as_i64() {
                MontyObject::int(value)
            } else if let Some(value) = value.as_f64() {
                MontyObject::float(value)
            } else {
                bail!("JSON number cannot be represented in Monty")
            }
        }
        Value::String(value) => MontyObject::string(value.clone()),
        Value::Array(values) => MontyObject::list(
            values
                .iter()
                .map(json_to_monty)
                .collect::<Result<Vec<_>>>()?,
        ),
        Value::Object(values) => MontyObject::dict(
            values
                .iter()
                .map(|(key, value)| Ok((MontyObject::string(key), json_to_monty(value)?)))
                .collect::<Result<Vec<_>>>()?,
        ),
    })
}

fn monty_to_json(value: ObjectRef<'_>) -> Value {
    if value.type_name() == "NoneType" {
        return Value::Null;
    }
    if let Some(value) = value.as_bool() {
        return Value::Bool(value);
    }
    if let Some(value) = value.as_int() {
        return Value::Number(value.into());
    }
    if let Some(value) = value.as_str() {
        return Value::String(value.to_string());
    }
    if let Some(value) = value.as_float()
        && value.is_finite()
    {
        return serde_json::Number::from_f64(value)
            .map(Value::Number)
            .unwrap_or(Value::Null);
    }
    if let Some(items) = value.items() {
        return Value::Array(items.into_iter().map(monty_to_json).collect());
    }
    if let Some(pairs) = value.pairs() {
        return pairs_to_json(pairs).unwrap_or_else(|| Value::String(value.py_repr()));
    }
    Value::String(value.py_repr())
}

fn pairs_to_json(pairs: Vec<(ObjectRef<'_>, ObjectRef<'_>)>) -> Option<Value> {
    let mut object = Map::new();
    for (key, value) in pairs {
        let key = key.as_str()?;
        object.insert(key.to_string(), monty_to_json(value));
    }
    Some(Value::Object(object))
}

fn format_success(
    stdout: &str,
    stderr: &str,
    value: &MontyObject,
    calls: &[CodeModeCall],
) -> String {
    format_output(
        "Code Mode completed",
        stdout,
        stderr,
        Some(&display_value(value)),
        calls,
    )
}

fn format_failure(
    stdout: &str,
    stderr: &str,
    error: &dyn std::fmt::Display,
    calls: &[CodeModeCall],
) -> String {
    format_output(
        &format!("Error: Code Mode failed: {error}"),
        stdout,
        stderr,
        None,
        calls,
    )
}

fn format_output(
    title: &str,
    stdout: &str,
    stderr: &str,
    value: Option<&str>,
    calls: &[CodeModeCall],
) -> String {
    let mut output = String::new();
    output.push_str(title);
    if !stdout.is_empty() {
        output.push_str("\n\nstdout:\n");
        output.push_str(stdout);
        if !stdout.ends_with('\n') {
            output.push('\n');
        }
    }
    if !stderr.is_empty() {
        output.push_str("\nstderr:\n");
        output.push_str(stderr);
        if !stderr.ends_with('\n') {
            output.push('\n');
        }
    }
    if let Some(value) = value {
        output.push_str("\nreturn:\n");
        output.push_str(value);
        output.push('\n');
    }
    output.push_str(&format!("\ntool calls: {}\n", calls.len()));
    for call in calls {
        let status = if call.succeeded {
            "completed"
        } else {
            "failed"
        };
        output.push_str(&format!(
            "- {}: {} ({} ms)\n",
            call.name, status, call.duration_ms
        ));
    }
    output.trim_end().to_string()
}

fn display_value(value: &MontyObject) -> String {
    if let Some(value) = value.as_ref().as_str() {
        value.to_string()
    } else {
        serde_json::to_string_pretty(&monty_to_json(value.as_ref()))
            .unwrap_or_else(|_| value.py_repr())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct TestHost {
        calls: Mutex<Vec<(String, Value)>>,
    }

    #[async_trait]
    impl CodeModeHost for TestHost {
        async fn call(&self, name: &str, arguments: Value) -> Result<Value> {
            self.calls
                .lock()
                .expect("test host mutex is not poisoned")
                .push((name.to_owned(), arguments.clone()));
            if name == "echo" {
                Ok(arguments)
            } else {
                Err(anyhow::anyhow!("unknown test tool: {name}"))
            }
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn runs_code_in_process_and_collects_output() {
        let runtime = CodeModeRuntime::new(CodeModeRuntimeConfig::default());
        let host = TestHost::default();
        let result = runtime
            .execute("print('hello')\n1 + 2", &[], &host)
            .await
            .expect("Code Mode execution should complete");

        assert!(!result.failed);
        assert!(result.output.contains("stdout:\nhello\n"));
        assert!(result.output.contains("return:\n3\n"));
        assert!(result.calls.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn resumes_monty_function_calls_with_json_values() {
        let runtime = CodeModeRuntime::new(CodeModeRuntimeConfig::default());
        let host = TestHost::default();
        let tools = vec![CodeModeTool {
            name: "echo".to_owned(),
            description: "Return the input value".to_owned(),
        }];
        let result = runtime
            .execute("echo({'value': 3})", &tools, &host)
            .await
            .expect("Code Mode execution should complete");

        assert!(!result.failed);
        assert_eq!(result.calls.len(), 1);
        assert_eq!(result.calls[0].name, "echo");
        assert!(result.calls[0].succeeded);
        assert_eq!(
            host.calls
                .lock()
                .expect("test host mutex is not poisoned")
                .as_slice(),
            &[("echo".to_owned(), serde_json::json!({"value": 3}))]
        );
        assert!(result.output.contains("\"value\": 3"));
    }
}
