/// GitHub Copilot API client (raw HTTP via reqwest)
///
/// API Documentation: https://docs.github.com/en/copilot
///
/// Endpoints:
/// - POST {api_base}/v1/chat/completions
///
/// ## Authentication
///
/// Copilot uses OAuth tokens instead of API keys. Tokens are obtained via:
/// 1. GitHub OAuth Device Code Flow
/// 2. Exchange GitHub token for Copilot API token
/// 3. Auto-refresh when expired
///
/// See `auth` module for token management.
use super::auth;
use super::types::{
    CopilotChoice, CopilotMessage, CopilotRequest, CopilotResponse, CopilotResponseFormat,
    CopilotResponseMessage, CopilotUsage,
};
use crate::apis::client::RateLimiter;
use crate::apis::llm::{
    ChatMessage, ChatRequest, ChatResponse, LlmClient, LlmError, MessageRole, Provider, Usage,
};
use crate::apis::stats::ApiStatsTracker;
use crate::logger::{self, LogTag};
use async_trait::async_trait;
use reqwest::Client;
use std::sync::Arc;
use std::time::{Duration, Instant};

// ============================================================================
// API CONFIGURATION
// ============================================================================

const ENDPOINT_CHAT: &str = "/chat/completions";
const DEFAULT_MODEL: &str = "gpt-4o";
const TIMEOUT_SECS: u64 = 30;
const DEFAULT_RATE_LIMIT_PER_MINUTE: usize = 50;

// Headers required by Copilot API (updated versions from ericc-ch/copilot-api)
const COPILOT_VERSION: &str = "0.26.7";
const HEADER_EDITOR_VERSION: &str = "vscode/1.100.0";
const HEADER_EDITOR_PLUGIN_VERSION: &str = "copilot-chat/0.26.7";
const HEADER_COPILOT_INTEGRATION_ID: &str = "vscode-chat";
const HEADER_USER_AGENT: &str = "GitHubCopilotChat/0.26.7";
const HEADER_API_VERSION: &str = "2025-04-01";

// ============================================================================
// CLIENT IMPLEMENTATION
// ============================================================================

/// GitHub Copilot API client
pub struct CopilotClient {
    client: Client,
    model: String,
    timeout: Duration,
    rate_limiter: RateLimiter,
    stats: Arc<ApiStatsTracker>,
    enabled: bool,
}

impl CopilotClient {
    /// Create a new Copilot client
    ///
    /// # Arguments
    /// * `model` - Optional model override (defaults to "gpt-4o")
    /// * `enabled` - Whether the client is enabled
    ///
    /// # Note
    /// No API key is required - Copilot uses OAuth tokens that are obtained
    /// via the `auth` module and refreshed automatically.
    pub fn new(model: Option<String>, enabled: bool) -> Self {
        Self {
            client: Client::new(),
            model: model.unwrap_or_else(|| DEFAULT_MODEL.to_string()),
            timeout: Duration::from_secs(TIMEOUT_SECS),
            rate_limiter: RateLimiter::new(DEFAULT_RATE_LIMIT_PER_MINUTE),
            stats: Arc::new(ApiStatsTracker::new()),
            enabled,
        }
    }

    /// Check if the user is authenticated with GitHub
    ///
    /// Returns true if a GitHub token exists (even if it might need refresh).
    pub fn is_authenticated() -> bool {
        auth::load_github_token().is_some()
    }

    /// Convert unified ChatRequest to Copilot-specific format
    fn build_copilot_request(&self, request: ChatRequest) -> CopilotRequest {
        let messages = request
            .messages
            .into_iter()
            .map(|msg| CopilotMessage {
                role: match msg.role {
                    MessageRole::System => "system".to_string(),
                    MessageRole::User => "user".to_string(),
                    MessageRole::Assistant => "assistant".to_string(),
                },
                content: msg.content,
            })
            .collect();

        CopilotRequest {
            model: request.model,
            messages,
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            response_format: request
                .response_format
                .map(|rf| CopilotResponseFormat { type_: rf.type_ }),
        }
    }

    /// Convert Copilot response to unified ChatResponse
    fn parse_copilot_response(
        &self,
        response: CopilotResponse,
        latency_ms: f64,
    ) -> Result<ChatResponse, LlmError> {
        // Get the first choice
        let choice = response
            .choices
            .first()
            .ok_or_else(|| LlmError::InvalidResponse {
                provider: "copilot".to_string(),
                message: "No choices in response".to_string(),
            })?;

        Ok(ChatResponse::new(
            choice.message.content.clone(),
            Usage::new(
                response.usage.prompt_tokens,
                response.usage.completion_tokens,
            ),
            choice
                .finish_reason
                .clone()
                .unwrap_or_else(|| "stop".to_string()),
            response.model,
            latency_ms,
        ))
    }

    /// Execute the API call
    async fn execute_request(
        &self,
        request: CopilotRequest,
    ) -> Result<(CopilotResponse, f64), LlmError> {
        if !self.enabled {
            return Err(LlmError::ProviderDisabled {
                provider: "copilot".to_string(),
            });
        }

        // Get valid Copilot token (auto-refreshes if expired)
        let copilot_token =
            auth::get_valid_copilot_token()
                .await
                .map_err(|e| LlmError::AuthError {
                    provider: "copilot".to_string(),
                    message: format!("Failed to get Copilot token: {}", e),
                })?;

        // Acquire rate limiter
        let guard = self
            .rate_limiter
            .acquire()
            .await
            .map_err(|e| LlmError::NetworkError {
                provider: "copilot".to_string(),
                message: format!("Rate limiter error: {}", e),
            })?;

        // Build full URL with API base from token
        let url = format!("{}{}", copilot_token.api_base, ENDPOINT_CHAT);

        logger::debug(
            LogTag::Api,
            &format!(
                "[COPILOT] Calling chat completions: model={}",
                request.model
            ),
        );

        let start = Instant::now();
        let request_id = uuid::Uuid::new_v4().to_string();
        let response_result = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", copilot_token.token))
            .header("Content-Type", "application/json")
            .header("accept", "application/json")
            .header("copilot-integration-id", HEADER_COPILOT_INTEGRATION_ID)
            .header("editor-version", HEADER_EDITOR_VERSION)
            .header("editor-plugin-version", HEADER_EDITOR_PLUGIN_VERSION)
            .header("user-agent", HEADER_USER_AGENT)
            .header("openai-intent", "conversation-panel")
            .header("x-github-api-version", HEADER_API_VERSION)
            .header("x-request-id", &request_id)
            .json(&request)
            .timeout(self.timeout)
            .send()
            .await;

        drop(guard);
        let elapsed = start.elapsed().as_millis() as f64;

        let mut response = response_result.map_err(|e| {
            if e.is_timeout() {
                LlmError::Timeout {
                    provider: "copilot".to_string(),
                    timeout_ms: self.timeout.as_millis() as u64,
                }
            } else {
                LlmError::NetworkError {
                    provider: "copilot".to_string(),
                    message: format!("Request failed: {}", e),
                }
            }
        })?;

        let status = response.status();

        // Handle error status codes
        if !status.is_success() {
            // Parse retry-after header BEFORE consuming body
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.parse::<u64>().ok())
                .map(|s| s * 1000); // Convert seconds to ms

            let error_body = response.text().await.unwrap_or_default();

            return Err(match status.as_u16() {
                401 | 403 => LlmError::AuthError {
                    provider: "copilot".to_string(),
                    message: "Invalid or expired Copilot token. Try re-authenticating.".to_string(),
                },
                429 => LlmError::RateLimited {
                    provider: "copilot".to_string(),
                    retry_after_ms: retry_after,
                },
                _ => LlmError::ApiError {
                    provider: "copilot".to_string(),
                    status_code: status.as_u16(),
                    message: error_body,
                },
            });
        }

        // Parse successful response
        let copilot_response =
            response
                .json::<CopilotResponse>()
                .await
                .map_err(|e| LlmError::ParseError {
                    provider: "copilot".to_string(),
                    message: format!("Failed to parse response: {}", e),
                })?;

        Ok((copilot_response, elapsed))
    }
}

#[async_trait]
impl LlmClient for CopilotClient {
    fn provider(&self) -> Provider {
        Provider::Copilot
    }

    fn is_enabled(&self) -> bool {
        self.enabled
    }

    async fn call(&self, request: ChatRequest) -> Result<ChatResponse, LlmError> {
        // Use the model from request, or fallback to client's default
        let mut request = request;
        if request.model.is_empty() {
            request.model = self.model.clone();
        }

        // Build Copilot-specific request
        let copilot_request = self.build_copilot_request(request);

        // Execute the request
        let (copilot_response, latency_ms) = match self.execute_request(copilot_request).await {
            Ok((resp, lat)) => {
                self.stats.record_request(true, lat).await;
                (resp, lat)
            }
            Err(e) => {
                self.stats.record_request(false, 0.0).await;
                self.stats
                    .record_error_with_event("Copilot", "chat_completion", format!("{}", e))
                    .await;
                return Err(e);
            }
        };

        // Parse and convert response
        self.parse_copilot_response(copilot_response, latency_ms)
    }

    async fn get_stats(&self) -> crate::apis::stats::ApiStats {
        self.stats.get_stats().await
    }

    fn rate_limit_info(&self) -> (usize, Duration) {
        (
            self.rate_limiter.max_per_minute(),
            self.rate_limiter.min_interval(),
        )
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apis::llm::ChatMessage;

    #[test]
    fn test_client_creation() {
        let client = CopilotClient::new(Some("gpt-4".to_string()), true);
        assert_eq!(client.model, "gpt-4");
        assert!(client.is_enabled());
    }

    #[test]
    fn test_client_creation_with_defaults() {
        let client = CopilotClient::new(None, true);
        assert_eq!(client.model, DEFAULT_MODEL);
    }

    #[test]
    fn test_build_copilot_request() {
        let client = CopilotClient::new(None, true);

        let request = ChatRequest::new(
            "gpt-4",
            vec![
                ChatMessage::system("You are helpful"),
                ChatMessage::user("Hello"),
            ],
        )
        .with_temperature(0.7)
        .with_max_tokens(100);

        let copilot_req = client.build_copilot_request(request);

        assert_eq!(copilot_req.model, "gpt-4");
        assert_eq!(copilot_req.messages.len(), 2);
        assert_eq!(copilot_req.messages[0].role, "system");
        assert_eq!(copilot_req.messages[1].role, "user");
        assert_eq!(copilot_req.temperature, Some(0.7));
        assert_eq!(copilot_req.max_tokens, Some(100));
    }

    #[test]
    fn test_provider() {
        let client = CopilotClient::new(None, true);
        assert_eq!(client.provider(), Provider::Copilot);
    }
}
