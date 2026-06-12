use std::sync::Arc;

use futures::FutureExt;
use serde_json::json;
use tokio::sync::Notify;

use crate::agent_events::{AgentEvent, ToolCall, ToolDefinition};
use crate::agent_tools;
use crate::ai::{self, AiCompletionRequest, AiConfig, AiMessage, AiProvider, AiStreamChunk};
use crate::connection::AppState;
use crate::models::connection::DatabaseType;
use tokio::sync::Mutex;

/// Maximum number of agent loop turns to prevent infinite loops.
const MAX_AGENT_TURNS: u32 = 10;

/// Context for an agent loop run.
pub struct AgentLoopContext {
    pub state: Arc<AppState>,
    pub connection_id: String,
    pub database: String,
    pub db_type: DatabaseType,
}

/// Check if the provider supports function calling / tool use.
/// Returns false for providers that are known to lack reliable tool support.
fn provider_supports_function_calling(config: &AiConfig) -> bool {
    match config.provider {
        // Ollama function calling support varies by model/version; conservative default is false.
        // Users with capable models can override via openai-compatible with an Ollama endpoint.
        AiProvider::Ollama => false,
        _ => true,
    }
}

/// Run the agent loop: call LLM with tools, execute tool calls, feed results back, repeat.
///
/// The `on_event` callback receives streaming events for the frontend.
/// Returns the final accumulated assistant text.
///
/// If the provider does not support function calling (e.g., Ollama), automatically
/// degrades to a text-only completion with schema context injected into the system prompt.
pub async fn run_agent_loop(
    config: &AiConfig,
    system_prompt: &str,
    messages: &[AiMessage],
    agent_ctx: &AgentLoopContext,
    on_event: impl Fn(AgentEvent) + Send + Sync + Clone + 'static,
    cancelled: &Notify,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
) -> Result<String, String> {
    // Auto-degrade: providers without function calling fall back to text-only completion.
    if !provider_supports_function_calling(config) {
        return run_agent_loop_text_only(
            config,
            system_prompt,
            messages,
            agent_ctx,
            on_event,
            cancelled,
            max_tokens,
            temperature,
        )
        .await;
    }
    let tools = agent_tools::read_only_tools();
    let mut conversation_messages: Vec<AiMessage> = messages.to_vec();
    let mut final_text = String::new();

    for turn in 0..MAX_AGENT_TURNS {
        // Check for cancellation before each turn
        if cancelled.notified().now_or_never().is_some() {
            on_event(AgentEvent::Error { message: "Agent loop cancelled".to_string() });
            break;
        }

        on_event(AgentEvent::TurnStart { turn });

        // Build the LLM request with tools
        let request =
            build_tool_request(config, system_prompt, &conversation_messages, &tools, max_tokens, temperature);

        // Stream the LLM response, collecting text and tool_calls
        let accumulated_text = Arc::new(Mutex::new(String::new()));
        let session_id = format!("agent-turn-{turn}");

        let acc = accumulated_text.clone();
        let on_event2 = on_event.clone();
        let on_chunk = move |chunk: AiStreamChunk| {
            if !chunk.delta.is_empty() {
                if let Ok(mut text) = acc.try_lock() {
                    text.push_str(&chunk.delta);
                }
                on_event2(AgentEvent::TextDelta { delta: chunk.delta.clone() });
            }
            if let Some(ref reasoning) = chunk.reasoning_delta {
                on_event2(AgentEvent::ReasoningDelta { delta: reasoning.clone() });
            }
        };

        // Call the LLM with tool support
        let collected_tool_calls = stream_with_tools(config, &request, &session_id, cancelled, on_chunk).await?;

        on_event(AgentEvent::TurnEnd { turn });

        let accumulated_text = accumulated_text.lock().await.clone();

        // Add assistant message to conversation (including tool_use blocks)
        conversation_messages.push(AiMessage {
            role: "assistant".to_string(),
            content: accumulated_text.clone(),
            tool_call_id: None,
            tool_calls: collected_tool_calls
                .iter()
                .map(|tc| ai::ToolCallRef { id: tc.id.clone(), name: tc.name.clone(), arguments: tc.arguments.clone() })
                .collect(),
        });

        if collected_tool_calls.is_empty() {
            // No tool calls -- we're done
            final_text = accumulated_text;
            break;
        }

        // Execute each tool call
        for tc in &collected_tool_calls {
            on_event(AgentEvent::ToolCallStart {
                tool_call_id: tc.id.clone(),
                tool_name: tc.name.clone(),
                args: tc.arguments.clone(),
            });

            let result = agent_tools::execute_tool(
                tc,
                &agent_ctx.state,
                &agent_ctx.connection_id,
                &agent_ctx.database,
                &agent_ctx.db_type,
            )
            .await;

            on_event(AgentEvent::ToolCallEnd {
                tool_call_id: tc.id.clone(),
                tool_name: tc.name.clone(),
                result: json!({ "content": result.content }),
                is_error: result.is_error,
            });

            // Add tool result to conversation for the next LLM call
            // Uses "tool" role per OpenAI convention; provider-specific conversion
            // happens in call_*_with_tools functions when building provider API requests.
            conversation_messages.push(AiMessage {
                role: "tool".to_string(),
                content: result.content.clone(),
                tool_call_id: Some(tc.id.clone()),
                tool_calls: Vec::new(),
            });
        }

        final_text = accumulated_text;
    }

    on_event(AgentEvent::AgentEnd { total_tokens: None });
    Ok(final_text)
}

/// Build an LLM request that includes tool definitions.
fn build_tool_request(
    config: &AiConfig,
    system_prompt: &str,
    messages: &[AiMessage],
    _tools: &[ToolDefinition], // Tools are injected in stream_with_tools / call_*_with_tools, not via AiCompletionRequest.
    max_tokens: Option<u32>,
    temperature: Option<f32>,
) -> AiCompletionRequest {
    // Note: tools are passed via the body, not via AiCompletionRequest.
    // The actual injection happens in stream_with_tools.
    AiCompletionRequest {
        config: config.clone(),
        system_prompt: system_prompt.to_string(),
        messages: messages.to_vec(),
        max_tokens: max_tokens.or(Some(4096)),
        temperature: temperature.or(Some(0.2)),
    }
}

/// Collect an LLM response with tool support, parsing tool_calls.
///
/// Named "stream" for consistency with the agent loop's streaming-API contract,
/// but currently uses a non-streaming (complete) LLM call internally, then emits
/// the full text as a single delta. True streaming tool_calls is deferred to
/// Phase 2 (provider-specific delta accumulation).
///
/// Check `cancelled` before the LLM call; if the LLM request has already started
/// it will run to completion (non-streaming requests cannot be interrupted mid-flight).
async fn stream_with_tools(
    config: &AiConfig,
    request: &AiCompletionRequest,
    session_id: &str,
    cancelled: &Notify,
    on_chunk: impl Fn(AiStreamChunk) + Send + Sync + 'static,
) -> Result<Vec<ToolCall>, String> {
    let tools = agent_tools::read_only_tools();

    // Return early if the user cancelled before the LLM call started.
    if cancelled.notified().now_or_never().is_some() {
        return Err("Agent loop cancelled".to_string());
    }

    // TODO(agent-loop-phase2): Implement streaming tool_calls for each provider.
    // Ref: .trellis/tasks/06-11-feat-ai-agent-loop-tool-calling-upgrade PRD Phase 2
    let client = ai::build_ai_http_client(config, 120)?;
    let response_text = call_with_tools(&client, config, request, &tools).await?;

    // Emit the full text as a single delta for the UI
    if !response_text.text.is_empty() {
        on_chunk(AiStreamChunk {
            session_id: session_id.to_string(),
            delta: response_text.text.clone(),
            reasoning_delta: None,
            done: false,
        });
    }

    on_chunk(AiStreamChunk {
        session_id: session_id.to_string(),
        delta: String::new(),
        reasoning_delta: None,
        done: true,
    });

    Ok(response_text.tool_calls)
}

/// Response from an LLM call that may include tool calls.
struct LlmResponse {
    text: String,
    tool_calls: Vec<ToolCall>,
}

/// Make a non-streaming LLM call with tools and parse the response.
async fn call_with_tools(
    client: &reqwest::Client,
    config: &AiConfig,
    request: &AiCompletionRequest,
    tools: &[ToolDefinition],
) -> Result<LlmResponse, String> {
    match config.provider {
        AiProvider::Claude => call_claude_with_tools(client, config, request, tools).await,
        AiProvider::Gemini => call_gemini_with_tools(client, config, request, tools).await,
        _ => call_openai_with_tools(client, config, request, tools).await,
    }
}

/// OpenAI / compatible providers: non-streaming call with tools.
async fn call_openai_with_tools(
    client: &reqwest::Client,
    config: &AiConfig,
    request: &AiCompletionRequest,
    tools: &[ToolDefinition],
) -> Result<LlmResponse, String> {
    let headers = ai::maybe_bearer_headers(config)?;

    let mut messages = vec![json!({ "role": "system", "content": request.system_prompt })];
    messages.extend(request.messages.iter().map(|m| {
        let mut msg = json!({ "role": m.role, "content": m.content });
        if m.role == "tool" {
            if let Some(ref tc_id) = m.tool_call_id {
                msg["tool_call_id"] = json!(tc_id);
            }
        } else if m.role == "assistant" && !m.tool_calls.is_empty() {
            let calls: Vec<serde_json::Value> = m
                .tool_calls
                .iter()
                .map(|tc| {
                    json!({
                        "id": tc.id,
                        "type": "function",
                        "function": {
                            "name": tc.name,
                            "arguments": tc.arguments.to_string()
                        }
                    })
                })
                .collect();
            msg["tool_calls"] = json!(calls);
        }
        msg
    }));

    let tool_json: Vec<serde_json::Value> = tools.iter().map(|t| t.to_openai_tool()).collect();

    let mut body = json!({
        "model": config.model,
        "messages": messages,
        "max_tokens": request.max_tokens.unwrap_or(4096),
        "tools": tool_json,
        "tool_choice": "auto",
        "stream": false,
    });
    ai::add_temperature_if_supported(&mut body, request);

    // Responses API tool calling is deferred to a future phase.
    // Use the completions endpoint regardless of api_style for now.
    let endpoint = ai::resolve_endpoint(config);

    let res = client
        .post(&endpoint)
        .headers(headers)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("AI request failed: {e}"))?;

    if !res.status().is_success() {
        let data: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
        return Err(ai::extract_error(&data).unwrap_or_else(|| "API error".to_string()));
    }

    let data: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;

    let text = data["choices"].get(0).and_then(|c| c["message"]["content"].as_str()).unwrap_or("").to_string();

    let mut tool_calls = Vec::new();
    if let Some(calls) = data["choices"].get(0).and_then(|c| c["message"]["tool_calls"].as_array()) {
        for tc in calls {
            let id = tc["id"].as_str().unwrap_or_default().to_string();
            let name = tc["function"]["name"].as_str().unwrap_or_default().to_string();
            let args_str = tc["function"]["arguments"].as_str().unwrap_or("{}");
            let args: serde_json::Value = serde_json::from_str(args_str).unwrap_or(json!({}));
            tool_calls.push(ToolCall { id, name, arguments: args });
        }
    }

    Ok(LlmResponse { text, tool_calls })
}

/// Claude: non-streaming call with tools.
async fn call_claude_with_tools(
    client: &reqwest::Client,
    config: &AiConfig,
    request: &AiCompletionRequest,
    tools: &[ToolDefinition],
) -> Result<LlmResponse, String> {
    let headers = ai::claude_headers(config)?;

    let mut messages: Vec<serde_json::Value> = Vec::new();
    for m in &request.messages {
        if m.role == "tool" {
            // Convert tool results to Anthropic tool_result content blocks
            messages.push(json!({
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": m.tool_call_id.as_deref().unwrap_or_default(),
                    "content": m.content
                }]
            }));
        } else if m.role == "assistant" && !m.tool_calls.is_empty() {
            // Reconstruct assistant message with tool_use content blocks
            let mut content_blocks: Vec<serde_json::Value> = Vec::new();
            if !m.content.is_empty() {
                content_blocks.push(json!({
                    "type": "text",
                    "text": m.content
                }));
            }
            for tc in &m.tool_calls {
                content_blocks.push(json!({
                    "type": "tool_use",
                    "id": tc.id,
                    "name": tc.name,
                    "input": tc.arguments
                }));
            }
            messages.push(json!({
                "role": "assistant",
                "content": content_blocks
            }));
        } else {
            messages.push(json!({ "role": m.role, "content": m.content }));
        }
    }

    let tool_json: Vec<serde_json::Value> = tools.iter().map(|t| t.to_anthropic_tool()).collect();

    let body = json!({
        "model": config.model,
        "max_tokens": request.max_tokens.unwrap_or(4096),
        "system": request.system_prompt,
        "messages": messages,
        "tools": tool_json,
    });

    let res = client
        .post(ai::resolve_endpoint(config))
        .headers(headers)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("AI request failed: {e}"))?;

    if !res.status().is_success() {
        let data: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
        return Err(ai::extract_error(&data).unwrap_or_else(|| "API error".to_string()));
    }

    let data: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;

    let mut text = String::new();
    let mut tool_calls = Vec::new();

    if let Some(content) = data["content"].as_array() {
        for block in content {
            match block["type"].as_str() {
                Some("text") => {
                    if let Some(t) = block["text"].as_str() {
                        text.push_str(t);
                    }
                }
                Some("tool_use") => {
                    let id = block["id"].as_str().unwrap_or_default().to_string();
                    let name = block["name"].as_str().unwrap_or_default().to_string();
                    let args = block["input"].clone();
                    tool_calls.push(ToolCall { id, name, arguments: args });
                }
                _ => {}
            }
        }
    }

    Ok(LlmResponse { text, tool_calls })
}

/// Gemini: non-streaming call with tools.
async fn call_gemini_with_tools(
    client: &reqwest::Client,
    config: &AiConfig,
    request: &AiCompletionRequest,
    tools: &[ToolDefinition],
) -> Result<LlmResponse, String> {
    let mut contents: Vec<serde_json::Value> = Vec::new();
    for m in &request.messages {
        if m.role == "tool" {
            // Extract tool name from id "gemini-tc-<tool_name>-<idx>"
            let tool_name = m
                .tool_call_id
                .as_deref()
                .and_then(|s| s.strip_prefix("gemini-tc-"))
                .and_then(|s| s.rsplitn(2, '-').nth(1))
                .unwrap_or("unknown");
            contents.push(json!({
                "role": "user",
                "parts": [{
                    "functionResponse": {
                        "name": tool_name,
                        "response": { "content": m.content }
                    }
                }]
            }));
        } else if m.role == "assistant" && !m.tool_calls.is_empty() {
            let mut parts: Vec<serde_json::Value> = Vec::new();
            if !m.content.is_empty() {
                parts.push(json!({ "text": m.content }));
            }
            for tc in &m.tool_calls {
                parts.push(json!({
                    "functionCall": {
                        "name": tc.name,
                        "args": tc.arguments
                    }
                }));
            }
            contents.push(json!({
                "role": "model",
                "parts": parts
            }));
        } else {
            let role = if m.role == "assistant" { "model" } else { "user" };
            contents.push(json!({
                "role": role,
                "parts": [{ "text": m.content }]
            }));
        }
    }

    let tool_declarations: Vec<serde_json::Value> = tools.iter().map(|t| t.to_gemini_tool()).collect();

    let body = json!({
        "contents": contents,
        "systemInstruction": { "parts": [{ "text": request.system_prompt }] },
        "tools": [{ "functionDeclarations": tool_declarations }],
        "generationConfig": {
            "maxOutputTokens": request.max_tokens.unwrap_or(4096),
        }
    });

    let res = client
        .post(ai::resolve_endpoint(config))
        .query(&[("key", config.api_key.as_str())])
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("AI request failed: {e}"))?;

    if !res.status().is_success() {
        let data: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
        return Err(ai::extract_error(&data).unwrap_or_else(|| "API error".to_string()));
    }

    let data: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;

    let mut text = String::new();
    let mut tool_calls = Vec::new();

    if let Some(candidates) = data["candidates"].as_array() {
        if let Some(parts) = candidates[0]["content"]["parts"].as_array() {
            for part in parts {
                if let Some(t) = part["text"].as_str() {
                    text.push_str(t);
                }
                if let Some(fc) = part.get("functionCall") {
                    let name = fc["name"].as_str().unwrap_or_default().to_string();
                    let args = fc["args"].clone();
                    let id = format!("gemini-tc-{name}-{}", tool_calls.len());
                    tool_calls.push(ToolCall { id, name, arguments: args });
                }
            }
        }
    }

    Ok(LlmResponse { text, tool_calls })
}

/// Text-only fallback for providers that don't support function calling.
///
/// Injects database schema context into the system prompt so the LLM can still
/// give informed answers, then performs a single non-streaming completion.
async fn run_agent_loop_text_only(
    config: &AiConfig,
    system_prompt: &str,
    messages: &[AiMessage],
    agent_ctx: &AgentLoopContext,
    on_event: impl Fn(AgentEvent) + Send + Sync + 'static,
    _cancelled: &Notify,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
) -> Result<String, String> {
    // Build a schema-enriched system prompt so the LLM can answer schema questions
    // even without tool access.
    let enriched_prompt = build_schema_prompt(agent_ctx, system_prompt).await;

    let request = AiCompletionRequest {
        config: config.clone(),
        system_prompt: enriched_prompt,
        messages: messages.to_vec(),
        max_tokens: max_tokens.or(Some(4096)),
        temperature: temperature.or(Some(0.2)),
    };

    // Use a non-streaming completion as the simplest fallback.
    let result = ai::complete(&request).await?;

    on_event(AgentEvent::TextDelta { delta: result.clone() });
    on_event(AgentEvent::AgentEnd { total_tokens: None });
    Ok(result)
}

/// Build a system prompt enriched with database schema information
/// for text-only mode where the LLM cannot use tools.
async fn build_schema_prompt(agent_ctx: &AgentLoopContext, system_prompt: &str) -> String {
    let mut enriched = system_prompt.to_string();

    // Fetch real schema data using the same core functions the tools would use
    let tables_result = crate::schema::list_tables_core(
        &agent_ctx.state,
        &agent_ctx.connection_id,
        &agent_ctx.database,
        "",
        None,
        Some(50), // smaller limit for prompt injection
    )
    .await;

    match tables_result {
        Ok(tables) if !tables.is_empty() => {
            enriched.push_str("\n\n## Database Schema (for context — no tools available)\n");
            enriched.push_str(&format!("Database: {}\n", agent_ctx.database));
            enriched.push_str("Tables:\n");
            for t in &tables {
                enriched.push_str(&format!("  - {} ({})", t.name, t.table_type));
                if let Some(ref comment) = t.comment {
                    if !comment.trim().is_empty() {
                        enriched.push_str(&format!(" — {}", comment.trim()));
                    }
                }
                enriched.push('\n');
            }
        }
        _ => {
            enriched.push_str("\n\n(Note: Unable to load database schema for this request.)\n");
        }
    }

    enriched
}
