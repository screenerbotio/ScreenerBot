//! GitHub Copilot API client module
//!
//! This module provides OAuth authentication and API client for GitHub Copilot.
//!
//! ## Modules
//!
//! - `auth` - OAuth Device Code Flow and token management
//! - `client` - HTTP client implementation
//! - `types` - Request/response types

pub mod auth;
pub mod client;
pub mod types;

pub use auth::{
    exchange_for_copilot_token, get_copilot_token_path, get_github_token_path,
    get_valid_copilot_token, is_authenticated, load_copilot_token, load_github_token,
    poll_for_access_token, request_device_code, save_copilot_token, save_github_token,
    CopilotToken, DeviceCodeResponse,
};

pub use client::CopilotClient;

pub use types::{
    CopilotChoice, CopilotMessage, CopilotRequest, CopilotResponse, CopilotResponseFormat,
    CopilotResponseMessage, CopilotUsage,
};
