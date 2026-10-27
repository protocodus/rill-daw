use crate::{
    AgentConfig, Backend, Message, Provider, SYSTEM_PROMPT, ScriptedToolCall, ScriptedTurn,
};
use daw_tools::ToolSpec;
use reqwest::blocking::Client;
use reqwest::header::HeaderValue;
use serde_json::{Value, json};
use std::{io::Read, time::Duration};

const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;

pub(crate) struct HttpBackend {
    config: AgentConfig,
    client: Client,
}

impl HttpBackend {
    pub fn new(config: AgentConfig) -> Result<Self, String> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            // Never forward provider credentials to a redirected endpoint.
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "Could not initialize secure provider connection")?;
        Ok(Self { config, client })
    }
}

impl Backend for HttpBackend {
    fn turn(&mut self, messages: &[Message], tools: &[ToolSpec]) -> Result<ScriptedTurn, String> {
        let body = request_body(self.config.provider, &self.config.model, messages, tools);
        if body.to_string().len() > MAX_RESPONSE_BYTES {
            return Err("Assistant request exceeds context limits".into());
        }
        let key = self.config.api_key.expose_secret();
        let request = match self.config.provider {
            Provider::OpenAi => self
                .client
                .post("https://api.openai.com/v1/chat/completions")
                .bearer_auth(key),
            Provider::Anthropic => {
                let mut credential =
                    HeaderValue::from_str(key).map_err(|_| "Invalid provider API key")?;
                credential.set_sensitive(true);
                self.client
                    .post("https://api.anthropic.com/v1/messages")
                    .header("x-api-key", credential)
                    .header("anthropic-version", "2023-06-01")
            }
        };
        let response = request.json(&body).send().map_err(|error| {
            if error.is_timeout() {
                "Provider request timed out"
            } else {
                "Could not connect to provider; check network and API settings"
            }
        })?;
        let status = response.status();
        if !status.is_success() {
            // Provider response bodies may echo inputs, authorization, or context.
            return Err(match status.as_u16() {
                401 | 403 => "Provider rejected the API key or model access".into(),
                429 => "Provider rate or usage limit reached; try again later".into(),
                400 | 404 | 422 => {
                    "Provider rejected the model or request; check the selected model".into()
                }
                code => format!("Provider request failed (HTTP {code})"),
            });
        }
        let mut bytes = Vec::new();
        response
            .take(MAX_RESPONSE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Could not read provider response")?;
        if bytes.len() > MAX_RESPONSE_BYTES {
            return Err("Provider response exceeds size limit".into());
        }
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|_| "Provider returned invalid JSON")?;
        parse_response(self.config.provider, &value)
    }
}

fn request_body(
    provider: Provider,
    model: &str,
    messages: &[Message],
    tools: &[ToolSpec],
) -> Value {
    match provider {
        Provider::OpenAi => {
            let mut output = vec![json!({"role": "system", "content": SYSTEM_PROMPT})];
            for message in messages {
                output.push(match message {
                    Message::User(text) => json!({"role": "user", "content": text}),
                    Message::Assistant { text, calls } => {
                        let mut value = json!({"role": "assistant", "content": if text.is_empty() { Value::Null } else { json!(text) }});
                        if !calls.is_empty() {
                            value["tool_calls"] = Value::Array(calls.iter().map(|call| json!({"id": call.id, "type": "function", "function": {"name": call.name, "arguments": call.arguments.to_string()}})).collect());
                        }
                        value
                    }
                    Message::Tool { id, result, .. } => json!({"role": "tool", "tool_call_id": id, "content": result.to_string()}),
                });
            }
            json!({"model": model, "max_completion_tokens": 8192, "messages": output, "tools": tools.iter().map(|tool| json!({"type":"function", "function":{"name":tool.name,"description":tool.description,"parameters":tool.input_schema}})).collect::<Vec<_>>(), "parallel_tool_calls": false})
        }
        Provider::Anthropic => {
            let mut output: Vec<Value> = Vec::new();
            for message in messages {
                match message {
                    Message::User(text) => output
                        .push(json!({"role": "user", "content": [{"type": "text", "text": text}]})),
                    Message::Assistant { text, calls } => {
                        let mut content = Vec::new();
                        if !text.is_empty() {
                            content.push(json!({"type": "text", "text": text}));
                        }
                        content.extend(calls.iter().map(|call| json!({"type": "tool_use", "id": call.id, "name": call.name, "input": call.arguments})));
                        output.push(json!({"role": "assistant", "content": content}));
                    }
                    Message::Tool {
                        id,
                        result,
                        is_error,
                    } => {
                        let content = json!({"type": "tool_result", "tool_use_id": id, "content": result.to_string(), "is_error": is_error});
                        if let Some(last) = output.last_mut().filter(|last| last["role"] == "user")
                        {
                            last["content"]
                                .as_array_mut()
                                .expect("user content is array")
                                .push(content);
                        } else {
                            output.push(json!({"role": "user", "content": [content]}));
                        }
                    }
                }
            }
            json!({"model": model, "max_tokens": 8192, "system": SYSTEM_PROMPT, "messages": output, "tools": tools.iter().map(|tool| json!({"name":tool.name,"description":tool.description,"input_schema":tool.input_schema})).collect::<Vec<_>>(), "tool_choice": {"type": "auto", "disable_parallel_tool_use": true}})
        }
    }
}

fn parse_response(provider: Provider, value: &Value) -> Result<ScriptedTurn, String> {
    match provider {
        Provider::OpenAi => {
            let choice = value["choices"]
                .as_array()
                .and_then(|choices| choices.first())
                .ok_or("Provider returned no completion")?;
            if choice["finish_reason"] == "length" {
                return Err("Provider response was truncated; try a smaller edit".into());
            }
            if choice["finish_reason"] == "content_filter" {
                return Err("Provider declined this request".into());
            }
            let message = &choice["message"];
            if !message.is_object() {
                return Err("Provider returned an invalid completion".into());
            }
            if message["refusal"].is_string() {
                return Err("Provider declined this request".into());
            }
            let text = message["content"].as_str().unwrap_or_default().to_owned();
            let mut calls = Vec::new();
            if let Some(items) = message.get("tool_calls").filter(|items| !items.is_null()) {
                for call in items.as_array().ok_or("Invalid provider tool calls")? {
                    let id = call["id"]
                        .as_str()
                        .ok_or("Missing provider tool-call ID")?
                        .to_owned();
                    let name = call["function"]["name"]
                        .as_str()
                        .ok_or("Missing provider tool name")?
                        .to_owned();
                    let raw = call["function"]["arguments"]
                        .as_str()
                        .ok_or("Missing provider tool arguments")?;
                    let arguments = serde_json::from_str(raw)
                        .map_err(|_| "Provider returned invalid tool arguments")?;
                    calls.push(ScriptedToolCall {
                        id,
                        name,
                        arguments,
                    });
                }
            }
            Ok(ScriptedTurn { text, calls })
        }
        Provider::Anthropic => {
            if value["stop_reason"] == "max_tokens"
                || value["stop_reason"] == "model_context_window_exceeded"
            {
                return Err("Provider response was truncated; try a smaller edit".into());
            }
            let content = value["content"]
                .as_array()
                .ok_or("Provider returned invalid content")?;
            let mut text = String::new();
            let mut calls = Vec::new();
            for block in content {
                match block["type"].as_str() {
                    Some("text") => {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(
                            block["text"]
                                .as_str()
                                .ok_or("Provider returned invalid text")?,
                        );
                    }
                    Some("tool_use") => calls.push(ScriptedToolCall {
                        id: block["id"]
                            .as_str()
                            .ok_or("Missing provider tool-call ID")?
                            .to_owned(),
                        name: block["name"]
                            .as_str()
                            .ok_or("Missing provider tool name")?
                            .to_owned(),
                        arguments: block
                            .get("input")
                            .ok_or("Missing provider tool arguments")?
                            .clone(),
                    }),
                    // Extended thinking is not requested; ignore non-action blocks.
                    _ => {}
                }
            }
            Ok(ScriptedTurn { text, calls })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_adapters_keep_tool_ids_arguments_results_and_error_state() {
        let tools = daw_tools::tool_specs();
        let messages = vec![
            Message::User("Add a bass".into()),
            Message::Assistant {
                text: "I will create it.".into(),
                calls: vec![ScriptedToolCall {
                    id: "call_1".into(),
                    name: "create_midi_part".into(),
                    arguments: json!({"name": "Bass"}),
                }],
            },
            Message::Tool {
                id: "call_1".into(),
                result: json!({"error":"revision conflict"}),
                is_error: true,
            },
        ];
        let openai = request_body(Provider::OpenAi, "model", &messages, &tools);
        assert_eq!(openai["messages"][2]["tool_calls"][0]["id"], "call_1");
        assert_eq!(openai["messages"][3]["tool_call_id"], "call_1");
        let anthropic = request_body(Provider::Anthropic, "model", &messages, &tools);
        assert_eq!(anthropic["messages"][1]["content"][1]["id"], "call_1");
        assert_eq!(anthropic["messages"][2]["content"][0]["is_error"], true);
        assert_eq!(
            openai["tools"][0]["function"]["parameters"],
            anthropic["tools"][0]["input_schema"]
        );
    }

    #[test]
    fn parses_provider_tool_requests_and_rejects_truncation() {
        let openai = json!({"choices":[{"finish_reason":"tool_calls","message":{"content":null,"tool_calls":[{"id":"a","type":"function","function":{"name":"set_mix","arguments":"{\"gain\":0.5}"}}]}}]});
        let anthropic = json!({"stop_reason":"tool_use","content":[{"type":"text","text":"Adjusting"},{"type":"tool_use","id":"b","name":"set_mix","input":{"gain":0.5}}]});
        assert_eq!(
            parse_response(Provider::OpenAi, &openai).unwrap().calls[0].arguments["gain"],
            0.5
        );
        assert_eq!(
            parse_response(Provider::Anthropic, &anthropic)
                .unwrap()
                .calls[0]
                .name,
            "set_mix"
        );
        assert!(
            parse_response(
                Provider::Anthropic,
                &json!({"stop_reason":"max_tokens","content":[]})
            )
            .is_err()
        );
        assert!(parse_response(Provider::OpenAi, &json!({"choices":[]})).is_err());
        assert_eq!(
            parse_response(
                Provider::OpenAi,
                &json!({"choices":[{"message":{"content":"Done", "tool_calls":null}}]})
            )
            .unwrap()
            .text,
            "Done"
        );
    }

    #[test]
    fn anthropic_groups_parallel_tool_results_into_one_user_message() {
        let body = request_body(
            Provider::Anthropic,
            "model",
            &[
                Message::Tool {
                    id: "a".into(),
                    result: json!({}),
                    is_error: false,
                },
                Message::Tool {
                    id: "b".into(),
                    result: json!({}),
                    is_error: false,
                },
            ],
            &[],
        );
        assert_eq!(body["messages"].as_array().unwrap().len(), 1);
        assert_eq!(body["messages"][0]["content"].as_array().unwrap().len(), 2);
    }
}
