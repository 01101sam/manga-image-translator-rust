use anyhow::{anyhow, Context};
use serde_json::{json, Value};

use crate::{Backend, ThinkingStrength};

#[derive(Clone, Debug, PartialEq)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolResult {
    pub id: String,
    pub output: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConvItem {
    User(String),
    Assistant(Vec<ToolCall>),
    Tools(Vec<ToolResult>),
}

#[derive(Clone, Debug)]
pub struct Conversation {
    pub system: String,
    pub items: Vec<ConvItem>,
}

impl Conversation {
    pub fn new(system: String, user: String) -> Self {
        Self {
            system,
            items: vec![ConvItem::User(user)],
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ModelTurn {
    pub calls: Vec<ToolCall>,
}

pub fn openai_url(base: &str) -> String {
    join(base, "v1/responses")
}

pub fn anthropic_url(base: &str) -> String {
    join(base, "v1/messages")
}

fn join(base: &str, path: &str) -> String {
    format!("{}/{}", base.trim_end_matches('/'), path)
}

pub fn openai_headers(api_key: &str) -> Vec<(String, String)> {
    vec![("Authorization".into(), format!("Bearer {api_key}"))]
}

pub fn anthropic_headers(api_key: &str) -> Vec<(String, String)> {
    vec![
        ("x-api-key".into(), api_key.into()),
        ("anthropic-version".into(), "2023-06-01".into()),
    ]
}

pub fn openai_request(
    model: &str,
    conv: &Conversation,
    tools: &[ToolSpec],
    thinking: Option<ThinkingStrength>,
) -> Value {
    let mut input = Vec::new();
    for item in &conv.items {
        match item {
            ConvItem::User(text) => input.push(json!({
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": text}],
            })),
            ConvItem::Assistant(calls) => {
                for call in calls {
                    input.push(json!({
                        "type": "function_call",
                        "call_id": call.id,
                        "name": call.name,
                        "arguments": call.arguments.to_string(),
                    }));
                }
            }
            ConvItem::Tools(results) => {
                for r in results {
                    input.push(json!({
                        "type": "function_call_output",
                        "call_id": r.id,
                        "output": r.output,
                    }));
                }
            }
        }
    }
    let mut body = json!({
        "model": model,
        "instructions": conv.system,
        "input": input,
        "tools": tools.iter().map(|t| json!({
            "type": "function",
            "name": t.name,
            "description": t.description,
            "parameters": t.parameters,
        })).collect::<Vec<_>>(),
        "tool_choice": "auto",
    });
    if let Some(effort) = thinking {
        body["reasoning"] = json!({ "effort": effort.as_str() });
    }
    body
}

pub fn openai_parse(v: &Value) -> anyhow::Result<ModelTurn> {
    if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
        return Err(anyhow!("openai error: {err}"));
    }
    let output = v
        .get("output")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("openai response missing output"))?;
    let mut calls = Vec::new();
    for item in output {
        if item.get("type").and_then(Value::as_str) != Some("function_call") {
            continue;
        }
        let id = item
            .get("call_id")
            .or_else(|| item.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("function_call missing call_id"))?
            .to_owned();
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("function_call missing name"))?
            .to_owned();
        let arguments = parse_args(item.get("arguments").unwrap_or(&Value::Null))?;
        calls.push(ToolCall {
            id,
            name,
            arguments,
        });
    }
    Ok(ModelTurn { calls })
}

pub fn anthropic_request(
    model: &str,
    conv: &Conversation,
    tools: &[ToolSpec],
    thinking: Option<ThinkingStrength>,
) -> Value {
    let mut messages = Vec::new();
    for item in &conv.items {
        match item {
            ConvItem::User(text) => messages.push(json!({
                "role": "user",
                "content": text,
            })),
            ConvItem::Assistant(calls) => messages.push(json!({
                "role": "assistant",
                "content": calls.iter().map(|c| json!({
                    "type": "tool_use",
                    "id": c.id,
                    "name": c.name,
                    "input": c.arguments,
                })).collect::<Vec<_>>(),
            })),
            ConvItem::Tools(results) => messages.push(json!({
                "role": "user",
                "content": results.iter().map(|r| json!({
                    "type": "tool_result",
                    "tool_use_id": r.id,
                    "content": r.output,
                })).collect::<Vec<_>>(),
            })),
        }
    }
    let mut body = json!({
        "model": model,
        "max_tokens": 32768,
        "system": conv.system,
        "messages": messages,
        "tools": tools.iter().map(|t| json!({
            "name": t.name,
            "description": t.description,
            "input_schema": t.parameters,
        })).collect::<Vec<_>>(),
    });
    if let Some(effort) = thinking {
        body["thinking"] = json!({
            "type": "enabled",
            "budget_tokens": effort.budget_tokens(),
        });
    }
    body
}

pub fn anthropic_parse(v: &Value) -> anyhow::Result<ModelTurn> {
    if v.get("type").and_then(Value::as_str) == Some("error") {
        return Err(anyhow!("anthropic error: {v}"));
    }
    let content = v
        .get("content")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("anthropic response missing content"))?;
    let mut calls = Vec::new();
    for item in content {
        if item.get("type").and_then(Value::as_str) != Some("tool_use") {
            continue;
        }
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("tool_use missing id"))?
            .to_owned();
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("tool_use missing name"))?
            .to_owned();
        let arguments = item.get("input").cloned().unwrap_or(json!({}));
        calls.push(ToolCall {
            id,
            name,
            arguments,
        });
    }
    Ok(ModelTurn { calls })
}

pub fn encode_request(
    backend: Backend,
    model: &str,
    conv: &Conversation,
    tools: &[ToolSpec],
    thinking: Option<ThinkingStrength>,
) -> (String, Value) {
    match backend {
        Backend::OpenAi => (
            "v1/responses".into(),
            openai_request(model, conv, tools, thinking),
        ),
        Backend::Anthropic => (
            "v1/messages".into(),
            anthropic_request(model, conv, tools, thinking),
        ),
    }
}

pub fn decode_response(backend: Backend, v: &Value) -> anyhow::Result<ModelTurn> {
    match backend {
        Backend::OpenAi => openai_parse(v),
        Backend::Anthropic => anthropic_parse(v),
    }
}

fn parse_args(v: &Value) -> anyhow::Result<Value> {
    match v {
        Value::String(s) => serde_json::from_str(s).with_context(|| format!("bad arguments: {s}")),
        other => Ok(other.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> ToolSpec {
        ToolSpec {
            name: "finish",
            description: "done",
            parameters: json!({
                "type": "object",
                "properties": {"translation": {"type": "string"}},
                "required": ["translation"],
            }),
        }
    }

    fn conv() -> Conversation {
        Conversation::new("sys".into(), "user".into())
    }

    #[test]
    fn openai_request_shape() {
        let body = openai_request(
            "deepseek-v4-flash",
            &conv(),
            &[spec()],
            Some(ThinkingStrength::High),
        );
        assert_eq!(body["model"], "deepseek-v4-flash");
        assert_eq!(body["instructions"], "sys");
        assert_eq!(body["reasoning"]["effort"], "high");
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["name"], "finish");
        assert_eq!(body["input"][0]["type"], "message");
        assert_eq!(body["input"][0]["role"], "user");
    }

    #[test]
    fn openai_omits_reasoning_when_off() {
        let body = openai_request("m", &conv(), &[spec()], None);
        assert!(body.get("reasoning").is_none());
    }

    #[test]
    fn openai_parses_function_call() {
        let v = json!({
            "output": [{
                "type": "function_call",
                "call_id": "call_1",
                "name": "finish",
                "arguments": "{\"translation\":\"[{\\\"text\\\":\\\"hi\\\"}]\"}"
            }]
        });
        let turn = openai_parse(&v).unwrap();
        assert_eq!(turn.calls[0].name, "finish");
        assert_eq!(
            turn.calls[0].arguments["translation"],
            "[{\"text\":\"hi\"}]"
        );
    }

    #[test]
    fn openai_roundtrip_tool_result() {
        let mut c = conv();
        c.items.push(ConvItem::Assistant(vec![ToolCall {
            id: "call_1".into(),
            name: "note_list".into(),
            arguments: json!({}),
        }]));
        c.items.push(ConvItem::Tools(vec![ToolResult {
            id: "call_1".into(),
            output: "[]".into(),
        }]));
        let body = openai_request("m", &c, &[spec()], None);
        assert_eq!(body["input"][1]["type"], "function_call");
        assert_eq!(body["input"][2]["type"], "function_call_output");
        assert_eq!(body["input"][2]["output"], "[]");
    }

    #[test]
    fn anthropic_request_shape() {
        let body = anthropic_request(
            "deepseek-v4-pro",
            &conv(),
            &[spec()],
            Some(ThinkingStrength::Low),
        );
        assert_eq!(body["model"], "deepseek-v4-pro");
        assert_eq!(body["system"], "sys");
        assert_eq!(body["max_tokens"], 32768);
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 2048);
        assert_eq!(body["tools"][0]["name"], "finish");
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
        assert_eq!(body["messages"][0]["role"], "user");
    }

    #[test]
    fn anthropic_parses_tool_use() {
        let v = json!({
            "content": [{
                "type": "tool_use",
                "id": "toolu_1",
                "name": "web_search",
                "input": {"query": "asuka"}
            }]
        });
        let turn = anthropic_parse(&v).unwrap();
        assert_eq!(turn.calls[0].id, "toolu_1");
        assert_eq!(turn.calls[0].arguments["query"], "asuka");
    }

    #[test]
    fn anthropic_roundtrip_tool_result() {
        let mut c = conv();
        c.items.push(ConvItem::Assistant(vec![ToolCall {
            id: "toolu_1".into(),
            name: "note_read".into(),
            arguments: json!({"name": "a"}),
        }]));
        c.items.push(ConvItem::Tools(vec![ToolResult {
            id: "toolu_1".into(),
            output: "hello".into(),
        }]));
        let body = anthropic_request("m", &c, &[spec()], None);
        assert_eq!(body["messages"][1]["content"][0]["type"], "tool_use");
        assert_eq!(body["messages"][2]["content"][0]["type"], "tool_result");
        assert_eq!(body["messages"][2]["content"][0]["tool_use_id"], "toolu_1");
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn urls() {
        assert_eq!(
            openai_url("https://api.deepseek.com/"),
            "https://api.deepseek.com/v1/responses"
        );
        assert_eq!(
            anthropic_url("https://api.deepseek.com/anthropic"),
            "https://api.deepseek.com/anthropic/v1/messages"
        );
    }
}
