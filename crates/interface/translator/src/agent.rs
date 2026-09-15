use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(test)]
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, bail, Context};
use serde_json::{json, Value};

use crate::backend::{
    anthropic_headers, anthropic_url, decode_response, encode_request, openai_headers, openai_url,
    ConvItem, Conversation, ToolCall, ToolResult, ToolSpec,
};
use crate::{Backend, Language, LlmTranslator, Transport};

const MAX_ITERS_DEFAULT: usize = 24;

const PYTHON_NOTES_API: &str = r#"
import os, pathlib
_NOTES = pathlib.Path(os.environ["TRANSLATOR_NOTES_DIR"])
def note_create(name, content=""):
    p = _NOTES / name
    if p.exists():
        raise FileExistsError(name)
    p.write_text(content)
def note_list():
    return sorted(p.name for p in _NOTES.iterdir() if p.is_file())
def note_read(name):
    return (_NOTES / name).read_text()
def note_write(name, content):
    (_NOTES / name).write_text(content)
def note_delete(name):
    p = _NOTES / name
    if p.exists():
        p.unlink()
"#;

pub fn system_prompt(target: Language, web_search: bool) -> String {
    let lang = target.display_zh();
    let search = if web_search {
        "(如果开启了联网搜索) 你可以使用 web_search 工具对该角色进行进一步搜索了解更多相关故事背景\n"
    } else {
        ""
    };
    format!(
        "角色: 你是专业日文翻译者, N1 水平, 擅长动漫文化\n\
背景: given 日文/英文或日英混杂的 `ocr_json`, 里面的文字内容为某个图片的文字提取, tags 能辅助描述该图片的大概内容以及相关角色\n\
{search}\
任务: 透过充分的背景内容, 以及根据 `ocr_json` 的文字内容, 进行翻译, 翻译为 {lang}\n\
`ocr_json` 的文字内容可能排序错乱, 你可以先重新排序再进行翻译, 但翻译完后输出的 json 格式必须符合输入的内容\n\
在翻译完成后, 调用 finish 并提交翻译内容完成本次翻译工作"
    )
}

pub fn user_message(ocr_json: &str, tags: Option<&str>) -> String {
    match tags {
        Some(tags) if !tags.is_empty() => {
            format!("<tags>\n{tags}\n</tags>\n<ocr_json>\n{ocr_json}\n</ocr_json>")
        }
        _ => format!("<ocr_json>\n{ocr_json}\n</ocr_json>"),
    }
}

pub fn tool_specs(web_search: bool) -> Vec<ToolSpec> {
    let obj = |props: Value, required: &[&str]| {
        json!({
            "type": "object",
            "properties": props,
            "required": required,
        })
    };
    let name = json!({"type": "string", "description": "Note name"});
    let content = json!({"type": "string", "description": "Note body"});
    let mut tools = vec![
        ToolSpec {
            name: "note_create",
            description: "Create a temporary note. Fails if the name already exists.",
            parameters: obj(json!({"name": name, "content": content}), &["name"]),
        },
        ToolSpec {
            name: "note_list",
            description: "List temporary note names.",
            parameters: obj(json!({}), &[]),
        },
        ToolSpec {
            name: "note_read",
            description: "Read a temporary note.",
            parameters: obj(json!({"name": name}), &["name"]),
        },
        ToolSpec {
            name: "note_write",
            description: "Overwrite a temporary note.",
            parameters: obj(
                json!({"name": name, "content": content}),
                &["name", "content"],
            ),
        },
        ToolSpec {
            name: "note_delete",
            description: "Delete a temporary note.",
            parameters: obj(json!({"name": name}), &["name"]),
        },
        ToolSpec {
            name: "execute",
            description: "Run a Python script. Notes helpers: note_create/list/read/write/delete.",
            parameters: obj(
                json!({"script": {"type": "string", "description": "Python source"}}),
                &["script"],
            ),
        },
        ToolSpec {
            name: "finish",
            description: "Submit the final translation and end this job.",
            parameters: obj(
                json!({"translation": {"description": "Translated ocr_json; must match the input JSON shape."}}),
                &["translation"],
            ),
        },
    ];
    if web_search {
        tools.insert(
            6,
            ToolSpec {
                name: "web_search",
                description: "Search the web for character or story background.",
                parameters: obj(
                    json!({"query": {"type": "string", "description": "Search query"}}),
                    &["query"],
                ),
            },
        );
    }
    tools
}

struct Notes {
    dir: tempfile::TempDir,
}

impl Notes {
    fn new() -> anyhow::Result<Self> {
        Ok(Self {
            dir: tempfile::TempDir::new()?,
        })
    }

    fn path(&self, name: &str) -> anyhow::Result<PathBuf> {
        if name.is_empty()
            || name.contains('/')
            || name.contains('\\')
            || name.contains("..")
            || Path::new(name).file_name().and_then(|s| s.to_str()) != Some(name)
        {
            bail!("invalid note name");
        }
        Ok(self.dir.path().join(name))
    }

    fn create(&self, name: &str, content: &str) -> anyhow::Result<String> {
        let p = self.path(name)?;
        if p.exists() {
            bail!("note already exists: {name}");
        }
        fs::write(p, content)?;
        Ok("ok".into())
    }

    fn list(&self) -> anyhow::Result<String> {
        let mut names = fs::read_dir(self.dir.path())?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        names.sort();
        Ok(serde_json::to_string(&names)?)
    }

    fn read(&self, name: &str) -> anyhow::Result<String> {
        fs::read_to_string(self.path(name)?).with_context(|| format!("note not found: {name}"))
    }

    fn write(&self, name: &str, content: &str) -> anyhow::Result<String> {
        fs::write(self.path(name)?, content)?;
        Ok("ok".into())
    }

    fn delete(&self, name: &str) -> anyhow::Result<String> {
        let p = self.path(name)?;
        if p.exists() {
            fs::remove_file(p)?;
        }
        Ok("ok".into())
    }
}

fn arg_str(args: &Value, key: &str) -> anyhow::Result<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("missing string field {key}"))
}

fn execute_python(notes: &Notes, script: &str) -> anyhow::Result<String> {
    let mut file = tempfile::NamedTempFile::new()?;
    use std::io::Write;
    write!(file, "{PYTHON_NOTES_API}\n{script}")?;
    let out = Command::new("python3")
        .arg(file.path())
        .env("TRANSLATOR_NOTES_DIR", notes.dir.path())
        .output()
        .context("failed to spawn python3")?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.stderr.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&String::from_utf8_lossy(&out.stderr));
    }
    if !out.status.success() {
        text.push_str(&format!("\nexit {out:?}"));
    }
    if text.is_empty() {
        text = "(no output)".into();
    }
    Ok(text)
}

fn tavily_search(translator: &LlmTranslator, query: &str) -> anyhow::Result<String> {
    let key = translator
        .web_search_key
        .as_deref()
        .ok_or_else(|| anyhow!("web_search is not enabled"))?;
    let v = translator.transport.post_json(
        "https://api.tavily.com/search",
        &[
            ("Authorization".into(), format!("Bearer {key}")),
            ("Content-Type".into(), "application/json".into()),
        ],
        json!({"query": query, "max_results": 5}),
    )?;
    Ok(v.to_string())
}

fn dispatch(translator: &LlmTranslator, notes: &Notes, call: &ToolCall) -> anyhow::Result<String> {
    match call.name.as_str() {
        "note_create" => notes.create(
            &arg_str(&call.arguments, "name")?,
            call.arguments
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or(""),
        ),
        "note_list" => notes.list(),
        "note_read" => notes.read(&arg_str(&call.arguments, "name")?),
        "note_write" => notes.write(
            &arg_str(&call.arguments, "name")?,
            &arg_str(&call.arguments, "content")?,
        ),
        "note_delete" => notes.delete(&arg_str(&call.arguments, "name")?),
        "execute" => execute_python(notes, &arg_str(&call.arguments, "script")?),
        "web_search" => tavily_search(translator, &arg_str(&call.arguments, "query")?),
        other => bail!("unknown tool: {other}"),
    }
}

pub fn finish_translation(args: &Value) -> anyhow::Result<String> {
    match args.get("translation") {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(other) => Ok(serde_json::to_string(other)?),
        None => Err(anyhow!("finish missing translation")),
    }
}

fn emit_llm_json(round: usize, url: &str, request: &Value, response: Value, error: bool) {
    let mut payload = json!({
        "round": round,
        "url": url,
        "request": request,
        "response": response,
    });
    if error {
        payload["error"] = json!(true);
    }
    eprintln!("LLM_JSON {payload}");
}

fn complete(
    translator: &LlmTranslator,
    conv: &Conversation,
    tools: &[ToolSpec],
    round: usize,
) -> anyhow::Result<crate::backend::ModelTurn> {
    let thinking = translator.thinking.then_some(translator.thinking_strength);
    let (_path, body) =
        encode_request(translator.backend, &translator.model, conv, tools, thinking);
    let (url, headers) = match translator.backend {
        Backend::OpenAi => (
            openai_url(&translator.base_url),
            openai_headers(&translator.api_key),
        ),
        Backend::Anthropic => (
            anthropic_url(&translator.base_url),
            anthropic_headers(&translator.api_key),
        ),
    };
    if std::env::var_os("MIT_LLM_TRACE").is_some() {
        let result = translator.transport.post_json(&url, &headers, body.clone());
        match &result {
            Ok(resp) => emit_llm_json(round, &url, &body, resp.clone(), false),
            Err(e) => emit_llm_json(round, &url, &body, json!(e.to_string()), true),
        }
        decode_response(translator.backend, &result?)
    } else {
        let resp = translator.transport.post_json(&url, &headers, body)?;
        decode_response(translator.backend, &resp)
    }
}

pub fn run_agent(
    translator: &LlmTranslator,
    ocr_json: &str,
    tags: Option<&str>,
    target: Language,
) -> anyhow::Result<String> {
    let web = translator.web_search_key.is_some();
    let tools = tool_specs(web);
    let mut conv = Conversation::new(system_prompt(target, web), user_message(ocr_json, tags));
    let notes = Notes::new()?;
    let max_iters = if translator.max_iters == 0 {
        MAX_ITERS_DEFAULT
    } else {
        translator.max_iters
    };
    for round in 1..=max_iters {
        let turn = complete(translator, &conv, &tools, round)?;
        if turn.calls.is_empty() {
            bail!("model returned no tool calls; finish is required");
        }
        conv.items.push(ConvItem::Assistant(turn.calls.clone()));
        let mut results = Vec::new();
        for call in &turn.calls {
            if call.name == "finish" {
                return finish_translation(&call.arguments);
            }
            let output = match dispatch(translator, &notes, call) {
                Ok(s) => s,
                Err(e) => format!("error: {e}"),
            };
            results.push(ToolResult {
                id: call.id.clone(),
                output,
            });
        }
        conv.items.push(ConvItem::Tools(results));
    }
    bail!("model did not call finish within {max_iters} iterations")
}

pub struct UreqTransport;

impl Transport for UreqTransport {
    fn post_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: Value,
    ) -> anyhow::Result<Value> {
        let mut req = ureq::post(url);
        for (k, v) in headers {
            req = req.header(k, v);
        }
        match req
            .header("Content-Type", "application/json")
            .send(body.to_string())
        {
            Ok(mut resp) => {
                let text = resp.body_mut().read_to_string()?;
                serde_json::from_str(&text).with_context(|| format!("invalid json from {url}"))
            }
            Err(e) => Err(anyhow!("http error contacting {url}: {e}")),
        }
    }
}

#[cfg(test)]
#[derive(Default)]
pub struct ScriptedTransport {
    pub responses: Mutex<Vec<Value>>,
    pub requests: Mutex<Vec<(String, Value)>>,
}

#[cfg(test)]
impl ScriptedTransport {
    pub fn new(responses: Vec<Value>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses),
            requests: Mutex::new(Vec::new()),
        })
    }
}

#[cfg(test)]
impl Transport for ScriptedTransport {
    fn post_json(
        &self,
        url: &str,
        _headers: &[(String, String)],
        body: Value,
    ) -> anyhow::Result<Value> {
        self.requests.lock().unwrap().push((url.to_owned(), body));
        let mut q = self.responses.lock().unwrap();
        if q.is_empty() {
            bail!("scripted transport: no more responses");
        }
        Ok(q.remove(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LlmConfig, ThinkingStrength};

    fn llm(backend: Backend, responses: Vec<Value>) -> (LlmTranslator, Arc<ScriptedTransport>) {
        let transport = ScriptedTransport::new(responses);
        let t = LlmTranslator::from_config(LlmConfig {
            backend,
            base_url: "http://127.0.0.1".into(),
            api_key: "k".into(),
            model: "deepseek-v4-flash".into(),
            thinking: true,
            thinking_strength: ThinkingStrength::High,
            web_search_key: None,
            max_iters: 8,
        })
        .with_transport(transport.clone());
        (t, transport)
    }

    fn openai_call(id: &str, name: &str, args: Value) -> Value {
        json!({
            "output": [{
                "type": "function_call",
                "call_id": id,
                "name": name,
                "arguments": args.to_string()
            }]
        })
    }

    fn anthropic_call(id: &str, name: &str, args: Value) -> Value {
        json!({
            "content": [{
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": args
            }]
        })
    }

    #[test]
    fn finish_ends_openai_loop() {
        let (t, _) = llm(
            Backend::OpenAi,
            vec![openai_call(
                "c1",
                "finish",
                json!({"translation": "[{\"text\":\"你好\"}]"}),
            )],
        );
        let out = t
            .translate_blocking("[{\"text\":\"こんにちは\"}]", None, Language::Chinese)
            .unwrap();
        assert_eq!(out, "[{\"text\":\"你好\"}]");
    }

    #[test]
    fn notes_then_finish_anthropic() {
        let (t, transport) = llm(
            Backend::Anthropic,
            vec![
                anthropic_call(
                    "t1",
                    "note_create",
                    json!({"name": "memo", "content": "asuka"}),
                ),
                anthropic_call("t2", "note_read", json!({"name": "memo"})),
                anthropic_call("t3", "finish", json!({"translation": [{"text": "ok"}]})),
            ],
        );
        let out = run_agent(&t, "[{\"text\":\"a\"}]", Some("asuka"), Language::Chinese).unwrap();
        assert_eq!(out, "[{\"text\":\"ok\"}]");
        let reqs = transport.requests.lock().unwrap();
        assert!(reqs[0].0.ends_with("/v1/messages"));
        assert_eq!(reqs[0].1["thinking"]["type"], "enabled");
        let tools: Vec<_> = reqs[0].1["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(!tools.contains(&"web_search"));
        assert!(tools.contains(&"finish"));
        assert!(reqs[0].1["system"].as_str().unwrap().contains("简体中文"));
        assert!(!reqs[0].1["system"].as_str().unwrap().contains("web_search"));
        assert!(reqs[0].1["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("<tags>"));
    }

    #[test]
    fn never_finish_errors() {
        let (t, _) = llm(
            Backend::OpenAi,
            vec![
                openai_call("c1", "note_list", json!({})),
                openai_call("c2", "note_list", json!({})),
            ],
        );
        let t = t.with_max_iters(2);
        let err = run_agent(&t, "[]", None, Language::English).unwrap_err();
        assert!(err.to_string().contains("did not call finish"));
    }

    #[test]
    fn web_search_offered_only_with_key() {
        let transport = ScriptedTransport::new(vec![openai_call(
            "c1",
            "finish",
            json!({"translation": "[]"}),
        )]);
        let t = LlmTranslator::from_config(LlmConfig {
            backend: Backend::OpenAi,
            base_url: "http://127.0.0.1".into(),
            api_key: "k".into(),
            model: "m".into(),
            thinking: false,
            thinking_strength: ThinkingStrength::High,
            web_search_key: Some("tvly".into()),
            max_iters: 4,
        })
        .with_transport(transport.clone());
        run_agent(&t, "[]", None, Language::Japanese).unwrap();
        let req = &transport.requests.lock().unwrap()[0].1;
        let names: Vec<_> = req["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"web_search"));
        assert!(req.get("reasoning").is_none());
        assert!(req["instructions"].as_str().unwrap().contains("web_search"));
    }

    #[test]
    fn execute_can_read_notes() {
        if Command::new("python3")
            .arg("-c")
            .arg("print(1)")
            .output()
            .is_err()
        {
            return;
        }
        let (t, _) = llm(
            Backend::OpenAi,
            vec![
                openai_call("c1", "note_write", json!({"name": "n", "content": "hello"})),
                openai_call("c2", "execute", json!({"script": "print(note_read('n'))"})),
                openai_call("c3", "finish", json!({"translation": "[]"})),
            ],
        );
        run_agent(&t, "[]", None, Language::English).unwrap();
    }

    #[test]
    fn tool_specs_without_search() {
        let names: Vec<_> = tool_specs(false).into_iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            [
                "note_create",
                "note_list",
                "note_read",
                "note_write",
                "note_delete",
                "execute",
                "finish"
            ]
        );
    }
}
