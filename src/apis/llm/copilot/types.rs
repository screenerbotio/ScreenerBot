/// GitHub Copilot API request/response types
///
/// Copilot uses an OpenAI-compatible API format.
/// API Documentation: https://docs.github.com/en/copilot
use serde::{Deserialize, Serialize};

// ============================================================================
// REQUEST TYPES
// ============================================================================

/// Copilot Chat Completion Request
#[derive(Debug, Clone, Serialize)]
pub struct CopilotRequest {
    /// Model ID (e.g., "gpt-4o", "gpt-4", "gpt-3.5-turbo")
    pub model: String,

    /// Array of messages in the conversation
    pub messages: Vec<CopilotMessage>,

    /// Sampling temperature (0.0-2.0)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,

    /// Maximum tokens to generate
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,

    /// Response format (for JSON mode)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<CopilotResponseFormat>,
}

/// Message in Copilot format
#[derive(Debug, Clone, Serialize)]
pub struct CopilotMessage {
    /// Role: "system", "user", or "assistant"
    pub role: String,

    /// Message content
    pub content: String,
}

/// Response format specification
#[derive(Debug, Clone, Serialize)]
pub struct CopilotResponseFormat {
    /// Format type: "text" or "json_object"
    #[serde(rename = "type")]
    pub type_: String,
}

impl CopilotResponseFormat {
    /// Create JSON object format
    pub fn json_object() -> Self {
        Self {
            type_: "json_object".to_string(),
        }
    }

    /// Create text format (default)
    pub fn text() -> Self {
        Self {
            type_: "text".to_string(),
        }
    }
}

// ============================================================================
// RESPONSE TYPES
// ============================================================================

/// Copilot Chat Completion Response
#[derive(Debug, Clone, Deserialize)]
pub struct CopilotResponse {
    /// Unique identifier for the completion
    pub id: String,

    /// Model used for generation
    pub model: String,

    /// Array of completion choices
    pub choices: Vec<CopilotChoice>,

    /// Token usage statistics
    pub usage: CopilotUsage,
}

/// A single choice in the response
#[derive(Debug, Clone, Deserialize)]
pub struct CopilotChoice {
    /// Index of this choice
    pub index: usize,

    /// The generated message
    pub message: CopilotResponseMessage,

    /// Reason for stopping ("stop", "length", "content_filter", etc.)
    pub finish_reason: Option<String>,
}

/// Response message from the assistant
#[derive(Debug, Clone, Deserialize)]
pub struct CopilotResponseMessage {
    /// Role (always "assistant")
    pub role: String,

    /// Generated content
    pub content: String,
}

/// Token usage statistics
#[derive(Debug, Clone, Deserialize)]
pub struct CopilotUsage {
    /// Tokens in the prompt
    pub prompt_tokens: u32,

    /// Tokens in the completion
    pub completion_tokens: u32,

    /// Total tokens used
    pub total_tokens: u32,
}
