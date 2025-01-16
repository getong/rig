//! Perplexity API client and Rig integration
//!
//! # Example
//! ```
//! use rig::providers::perplexity;
//!
//! let client = perplexity::Client::new("YOUR_API_KEY");
//!
//! let llama_3_1_sonar_small_online = client.completion_model(perplexity::LLAMA_3_1_SONAR_SMALL_ONLINE);
//! ```

use crate::{
    agent::AgentBuilder,
    completion::{self, CompletionError},
    extractor::ExtractorBuilder,
    json_utils,
    message::{self, MessageError},
};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

// ================================================================
// Main Cohere Client
// ================================================================
const PERPLEXITY_API_BASE_URL: &str = "https://api.perplexity.ai";

#[derive(Clone)]
pub struct Client {
    base_url: String,
    http_client: reqwest::Client,
}

impl Client {
    pub fn new(api_key: &str) -> Self {
        Self::from_url(api_key, PERPLEXITY_API_BASE_URL)
    }

    /// Create a new Perplexity client from the `PERPLEXITY_API_KEY` environment variable.
    /// Panics if the environment variable is not set.
    pub fn from_env() -> Self {
        let api_key = std::env::var("PERPLEXITY_API_KEY").expect("PERPLEXITY_API_KEY not set");
        Self::new(&api_key)
    }

    pub fn from_url(api_key: &str, base_url: &str) -> Self {
        Self {
            base_url: base_url.to_string(),
            http_client: reqwest::Client::builder()
                .default_headers({
                    let mut headers = reqwest::header::HeaderMap::new();
                    headers.insert(
                        "Authorization",
                        format!("Bearer {}", api_key)
                            .parse()
                            .expect("Bearer token should parse"),
                    );
                    headers
                })
                .build()
                .expect("Perplexity reqwest client should build"),
        }
    }

    pub fn post(&self, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}/{}", self.base_url, path).replace("//", "/");
        self.http_client.post(url)
    }

    pub fn completion_model(&self, model: &str) -> CompletionModel {
        CompletionModel::new(self.clone(), model)
    }

    pub fn agent(&self, model: &str) -> AgentBuilder<CompletionModel> {
        AgentBuilder::new(self.completion_model(model))
    }

    pub fn extractor<T: JsonSchema + for<'a> Deserialize<'a> + Serialize + Send + Sync>(
        &self,
        model: &str,
    ) -> ExtractorBuilder<T, CompletionModel> {
        ExtractorBuilder::new(self.completion_model(model))
    }
}

#[derive(Debug, Deserialize)]
struct ApiErrorResponse {
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ApiResponse<T> {
    Ok(T),
    Err(ApiErrorResponse),
}

// ================================================================
// Perplexity Completion API
// ================================================================
/// `llama-3.1-sonar-small-128k-online` completion model
pub const LLAMA_3_1_SONAR_SMALL_ONLINE: &str = "llama-3.1-sonar-small-128k-online";
/// `llama-3.1-sonar-large-128k-online` completion model
pub const LLAMA_3_1_SONAR_LARGE_ONLINE: &str = "llama-3.1-sonar-large-128k-online";
/// `llama-3.1-sonar-huge-128k-online` completion model
pub const LLAMA_3_1_SONAR_HUGE_ONLINE: &str = "llama-3.1-sonar-huge-128k-online";
/// `llama-3.1-sonar-small-128k-chat` completion model
pub const LLAMA_3_1_SONAR_SMALL_CHAT: &str = "llama-3.1-sonar-small-128k-chat";
/// `llama-3.1-sonar-large-128k-chat` completion model
pub const LLAMA_3_1_SONAR_LARGE_CHAT: &str = "llama-3.1-sonar-large-128k-chat";
/// `llama-3.1-8b-instruct` completion model
pub const LLAMA_3_1_8B_INSTRUCT: &str = "llama-3.1-8b-instruct";
/// `llama-3.1-70b-instruct` completion model
pub const LLAMA_3_1_70B_INSTRUCT: &str = "llama-3.1-70b-instruct";

#[derive(Debug, Deserialize)]
pub struct CompletionResponse {
    pub id: String,
    pub model: String,
    pub object: String,
    pub created: u64,
    #[serde(default)]
    pub choices: Vec<Choice>,
    pub usage: Usage,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Deserialize, Debug)]
pub struct Delta {
    pub role: Role,
    pub content: String,
}

#[derive(Deserialize, Debug)]
pub struct Choice {
    pub index: usize,
    pub finish_reason: String,
    pub message: Message,
    pub delta: Delta,
}

#[derive(Deserialize, Debug)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

impl std::fmt::Display for Usage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Prompt tokens: {}\nCompletion tokens: {} Total tokens: {}",
            self.prompt_tokens, self.completion_tokens, self.total_tokens
        )
    }
}

impl TryFrom<CompletionResponse> for completion::CompletionResponse<CompletionResponse> {
    type Error = CompletionError;

    fn try_from(value: CompletionResponse) -> std::prelude::v1::Result<Self, Self::Error> {
        match value.choices.as_slice() {
            [Choice {
                message: Message { content, .. },
                ..
            }, ..] => Ok(completion::CompletionResponse {
                choice: completion::ModelChoice::Message(content.to_string()),
                raw_response: value,
            }),
            _ => Err(CompletionError::ResponseError(
                "Response did not contain a message or tool call".into(),
            )),
        }
    }
}

#[derive(Clone)]
pub struct CompletionModel {
    client: Client,
    pub model: String,
}

impl CompletionModel {
    pub fn new(client: Client, model: &str) -> Self {
        Self {
            client,
            model: model.to_string(),
        }
    }
}

impl TryFrom<message::Message> for Vec<Message> {
    type Error = MessageError;

    fn try_from(message: message::Message) -> Result<Self, Self::Error> {
        Ok(match message {
            message::Message::User { content } => content
                .into_iter()
                .map(|content| match content {
                    message::UserContent::Text { text } => Ok(Message {
                        role: Role::User,
                        content: text,
                    }),
                    _ => Err(MessageError::ConversionError(
                        "Only text content is supported by Perplexity".to_owned(),
                    )),
                })
                .collect::<Result<Vec<_>, _>>()?,

            message::Message::Assistant {
                content,
                tool_calls,
            } => {
                if tool_calls.len() > 0 {
                    return Err(MessageError::ConversionError(
                        "Tool calls are not supported by Perplexity".to_owned(),
                    ));
                }

                content
                    .into_iter()
                    .map(|content| Message {
                        role: Role::Assistant,
                        content: content.into(),
                    })
                    .collect::<Vec<_>>()
            }

            _ => {
                return Err(MessageError::ConversionError(
                    "Only user and assistant messages are supported by Perplexity".to_owned(),
                ))
            }
        })
    }
}

impl completion::CompletionModel for CompletionModel {
    type Response = CompletionResponse;

    async fn completion(
        &self,
        completion_request: completion::CompletionRequest,
    ) -> Result<completion::CompletionResponse<CompletionResponse>, CompletionError> {
        // Add context documents to chat history
        let prompt_with_context = completion_request.prompt_with_context();

        // Add preamble to messages (if available)
        let mut messages: Vec<Message> = if let Some(preamble) = completion_request.preamble {
            let message: message::Message = preamble.into();
            message
                .try_into()
                .map_err(|e: MessageError| CompletionError::RequestError(e.to_string().into()))?
        } else {
            vec![]
        };

        // Add chat history to messages
        for message in completion_request.chat_history {
            let converted: Vec<Message> = message
                .try_into()
                .map_err(|e: MessageError| CompletionError::RequestError(e.to_string().into()))?;
            messages.extend(converted);
        }

        // Add user prompt to messages
        let user_messages: Vec<Message> = prompt_with_context
            .try_into()
            .map_err(|e: MessageError| CompletionError::RequestError(e.to_string().into()))?;
        messages.extend(user_messages);

        // Compose request
        let request = json!({
            "model": self.model,
            "messages": messages,
            "temperature": completion_request.temperature,
        });

        let response = self
            .client
            .post("/chat/completions")
            .json(
                &if let Some(ref params) = completion_request.additional_params {
                    json_utils::merge(request.clone(), params.clone())
                } else {
                    request.clone()
                },
            )
            .send()
            .await?;

        if response.status().is_success() {
            match response.json::<ApiResponse<CompletionResponse>>().await? {
                ApiResponse::Ok(completion) => {
                    tracing::info!(target: "rig",
                        "Perplexity completion token usage: {}",
                        completion.usage
                    );
                    Ok(completion.try_into()?)
                }
                ApiResponse::Err(error) => Err(CompletionError::ProviderError(error.message)),
            }
        } else {
            Err(CompletionError::ProviderError(response.text().await?))
        }
    }
}
