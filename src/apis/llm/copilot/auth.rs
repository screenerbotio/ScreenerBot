//! GitHub Copilot OAuth Authentication Module
//!
//! This module handles the OAuth Device Code Flow for GitHub Copilot:
//! 1. Request device code from GitHub
//! 2. User authorizes via browser
//! 3. Poll for GitHub access token
//! 4. Exchange for Copilot API token
//! 5. Cache tokens with auto-refresh
//!
//! ## OAuth Flow
//!
//! ```text
//! 1. request_device_code() → DeviceCodeResponse
//! 2. User visits verification_uri and enters user_code
//! 3. poll_for_access_token() → GitHub access token (loop with interval)
//! 4. exchange_for_copilot_token() → Copilot API token
//! 5. save_copilot_token() → Cache to disk
//! ```
//!
//! ## Token Management
//!
//! - GitHub tokens stored in `data/github_token.json`
//! - Copilot tokens stored in `data/copilot_token.json`
//! - Copilot tokens auto-refresh when expired
//!
//! ## Usage
//!
//! ```rust
//! // Get a valid Copilot token (auto-refresh if needed)
//! let token = get_valid_copilot_token().await?;
//! ```

use crate::logger::{self, LogTag};
use crate::paths::get_data_directory;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

// =============================================================================
// CONSTANTS
// =============================================================================

/// GitHub OAuth client ID for Copilot
const GITHUB_CLIENT_ID: &str = "Iv1.b507a08c87ecfe98";

/// GitHub device code endpoint
const DEVICE_CODE_URL: &str = "https://github.com/login/device/code";

/// GitHub access token endpoint
const ACCESS_TOKEN_URL: &str = "https://github.com/login/oauth/access_token";

/// Copilot token exchange endpoint
const COPILOT_TOKEN_URL: &str = "https://api.github.com/copilot_internal/v2/token";

/// Default Copilot API base URL
const DEFAULT_COPILOT_API_BASE: &str = "https://api.githubcopilot.com";

// =============================================================================
// TYPES
// =============================================================================

/// Response from GitHub device code request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceCodeResponse {
    /// Device code for polling
    pub device_code: String,
    /// User code to display to user
    pub user_code: String,
    /// URL where user should authorize
    pub verification_uri: String,
    /// Time until device code expires (seconds)
    pub expires_in: u64,
    /// Polling interval (seconds)
    pub interval: u64,
}

/// Copilot API token with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CopilotToken {
    /// The actual token string
    pub token: String,
    /// When the token expires (Unix timestamp in seconds)
    pub expires_at: u64,
    /// API base URL extracted from token
    pub api_base: String,
}

impl CopilotToken {
    /// Check if token is expired (with 5 minute buffer)
    pub fn is_expired(&self) -> bool {
        let now = current_timestamp();
        let buffer = 300; // 5 minutes
        now >= self.expires_at.saturating_sub(buffer)
    }
}

// Internal response structs
#[derive(Debug, Deserialize)]
struct AccessTokenResponse {
    access_token: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CopilotTokenResponse {
    token: String,
    expires_at: Option<u64>,
}

// =============================================================================
// PATH HELPERS
// =============================================================================

/// Get path to GitHub token storage file
pub fn get_github_token_path() -> PathBuf {
    get_data_directory().join("github_token.json")
}

/// Get path to Copilot token storage file
pub fn get_copilot_token_path() -> PathBuf {
    get_data_directory().join("copilot_token.json")
}

// =============================================================================
// OAUTH DEVICE CODE FLOW
// =============================================================================

/// Step 1: Request device code from GitHub
///
/// This initiates the OAuth device flow. The returned `DeviceCodeResponse`
/// contains the user code and verification URL to display to the user.
///
/// ## Returns
///
/// - `Ok(DeviceCodeResponse)` - Device code and user instructions
/// - `Err(String)` - HTTP or parsing error
pub async fn request_device_code() -> Result<DeviceCodeResponse, String> {
    logger::info(LogTag::Api, "[COPILOT] Requesting device code from GitHub");

    let client = Client::new();
    let params = [("client_id", GITHUB_CLIENT_ID), ("scope", "user:email")];

    let response = client
        .post(DEVICE_CODE_URL)
        .header("Accept", "application/json")
        .header("User-Agent", "GithubCopilot/1.155.0")
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Failed to request device code: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Device code request failed ({}): {}", status, body));
    }

    let device_code: DeviceCodeResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse device code response: {}", e))?;

    logger::info(
        LogTag::Api,
        &format!(
            "[COPILOT] Device code received. User code: {}",
            device_code.user_code
        ),
    );

    Ok(device_code)
}

/// Step 2: Poll for GitHub access token
///
/// Call this in a loop with the specified interval until user authorizes.
/// Returns `Ok(Some(token))` when authorized, `Ok(None)` when still pending.
///
/// ## Arguments
///
/// * `device_code` - Device code from `request_device_code()`
///
/// ## Returns
///
/// - `Ok(Some(token))` - User authorized, got access token
/// - `Ok(None)` - Still waiting for authorization
/// - `Err(String)` - Error occurred (expired, denied, network error)
pub async fn poll_for_access_token(device_code: &str) -> Result<Option<String>, String> {
    let client = Client::new();
    let params = [
        ("client_id", GITHUB_CLIENT_ID),
        ("device_code", device_code),
        ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
    ];

    let response = client
        .post(ACCESS_TOKEN_URL)
        .header("Accept", "application/json")
        .header("User-Agent", "GithubCopilot/1.155.0")
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("Failed to poll for access token: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Access token poll failed ({}): {}", status, body));
    }

    let token_response: AccessTokenResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse access token response: {}", e))?;

    // Check for errors in response
    if let Some(error) = token_response.error {
        match error.as_str() {
            "authorization_pending" => {
                // User hasn't authorized yet - this is normal
                return Ok(None);
            }
            "slow_down" => {
                return Err("Polling too frequently. Increase interval.".to_string());
            }
            "expired_token" => {
                return Err("Device code expired. Start over.".to_string());
            }
            "access_denied" => {
                return Err("User denied authorization.".to_string());
            }
            _ => {
                return Err(format!("GitHub OAuth error: {}", error));
            }
        }
    }

    // Success - we got an access token
    if let Some(access_token) = token_response.access_token {
        logger::info(LogTag::Api, "[COPILOT] GitHub access token acquired");
        return Ok(Some(access_token));
    }

    // No error and no token - treat as pending
    Ok(None)
}

/// Step 3: Exchange GitHub token for Copilot API token
///
/// Uses the GitHub access token to get a Copilot-specific API token.
///
/// ## Arguments
///
/// * `github_token` - GitHub access token from `poll_for_access_token()`
///
/// ## Returns
///
/// - `Ok(CopilotToken)` - Copilot token with expiry and API base
/// - `Err(String)` - HTTP or parsing error
pub async fn exchange_for_copilot_token(github_token: &str) -> Result<CopilotToken, String> {
    logger::info(
        LogTag::Api,
        "[COPILOT] Exchanging GitHub token for Copilot token",
    );

    let client = Client::new();

    let response = client
        .get(COPILOT_TOKEN_URL)
        .header("Authorization", format!("token {}", github_token))
        .header("Accept", "application/json")
        .header("User-Agent", "GithubCopilot/1.155.0")
        .send()
        .await
        .map_err(|e| format!("Failed to exchange for Copilot token: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "Copilot token exchange failed ({}): {}",
            status, body
        ));
    }

    let copilot_response: CopilotTokenResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Copilot token response: {}", e))?;

    // Parse the token to extract API base URL
    let api_base = parse_api_base_from_token(&copilot_response.token)
        .unwrap_or_else(|| DEFAULT_COPILOT_API_BASE.to_string());

    let expires_at = copilot_response
        .expires_at
        .unwrap_or_else(|| current_timestamp() + 1800); // Default 30 min

    logger::info(
        LogTag::Api,
        &format!("[COPILOT] Copilot token acquired. API base: {}", api_base),
    );

    Ok(CopilotToken {
        token: copilot_response.token,
        expires_at,
        api_base,
    })
}

// =============================================================================
// TOKEN STORAGE
// =============================================================================

/// Check if user is authenticated with GitHub Copilot
///
/// Returns true if a GitHub access token is saved (from previous OAuth flow).
/// The Copilot API token can always be refreshed from the GitHub token.
pub fn is_authenticated() -> bool {
    let path = get_github_token_path();
    path.exists()
}

/// Load saved GitHub access token
///
/// ## Returns
///
/// - `Some(token)` - Token found in storage
/// - `None` - No token stored or file unreadable
pub fn load_github_token() -> Option<String> {
    let path = get_github_token_path();

    if !path.exists() {
        return None;
    }

    let contents = std::fs::read_to_string(&path).ok()?;
    let token: String = serde_json::from_str(&contents).ok()?;

    logger::debug(LogTag::Api, "[COPILOT] Loaded GitHub token from storage");

    Some(token)
}

/// Save GitHub access token to storage
///
/// ## Arguments
///
/// * `token` - GitHub access token to save
///
/// ## Returns
///
/// - `Ok(())` - Token saved successfully
/// - `Err(String)` - File write error
pub fn save_github_token(token: &str) -> Result<(), String> {
    let path = get_github_token_path();

    let json = serde_json::to_string_pretty(token)
        .map_err(|e| format!("Failed to serialize GitHub token: {}", e))?;

    std::fs::write(&path, json)
        .map_err(|e| format!("Failed to write GitHub token to {}: {}", path.display(), e))?;

    logger::info(
        LogTag::Api,
        &format!("[COPILOT] Saved GitHub token to {}", path.display()),
    );

    Ok(())
}

/// Load cached Copilot token (returns None if expired)
///
/// ## Returns
///
/// - `Some(token)` - Valid (non-expired) token found
/// - `None` - No token stored, unreadable, or expired
pub fn load_copilot_token() -> Option<CopilotToken> {
    let path = get_copilot_token_path();

    if !path.exists() {
        return None;
    }

    let contents = std::fs::read_to_string(&path).ok()?;
    let token: CopilotToken = serde_json::from_str(&contents).ok()?;

    if token.is_expired() {
        logger::debug(LogTag::Api, "[COPILOT] Cached Copilot token is expired");
        return None;
    }

    logger::debug(
        LogTag::Api,
        "[COPILOT] Loaded valid Copilot token from cache",
    );

    Some(token)
}

/// Save Copilot token to storage
///
/// ## Arguments
///
/// * `token` - Copilot token to save
///
/// ## Returns
///
/// - `Ok(())` - Token saved successfully
/// - `Err(String)` - File write error
pub fn save_copilot_token(token: &CopilotToken) -> Result<(), String> {
    let path = get_copilot_token_path();

    let json = serde_json::to_string_pretty(token)
        .map_err(|e| format!("Failed to serialize Copilot token: {}", e))?;

    std::fs::write(&path, json)
        .map_err(|e| format!("Failed to write Copilot token to {}: {}", path.display(), e))?;

    logger::info(
        LogTag::Api,
        &format!("[COPILOT] Saved Copilot token to {}", path.display()),
    );

    Ok(())
}

// =============================================================================
// HIGH-LEVEL API
// =============================================================================

/// Get a valid Copilot token, refreshing if needed
///
/// This is the main entry point for getting a Copilot token. It:
/// 1. Checks for cached token (returns if valid)
/// 2. Loads GitHub token from storage
/// 3. Exchanges for new Copilot token
/// 4. Caches the new token
///
/// ## Returns
///
/// - `Ok(CopilotToken)` - Valid token ready to use
/// - `Err(String)` - No GitHub token available or exchange failed
///
/// ## Note
///
/// If this fails with "No GitHub token", you need to run the OAuth flow first:
/// ```rust
/// let device_code = request_device_code().await?;
/// // Display device_code.verification_uri and device_code.user_code to user
/// // Poll until authorized
/// let github_token = poll_for_access_token(&device_code.device_code).await?;
/// save_github_token(&github_token)?;
/// ```
pub async fn get_valid_copilot_token() -> Result<CopilotToken, String> {
    // Try to use cached token
    if let Some(token) = load_copilot_token() {
        return Ok(token);
    }

    // Need to refresh - get GitHub token
    let github_token = load_github_token()
        .ok_or_else(|| "No GitHub token available. Please authenticate first.".to_string())?;

    // Exchange for new Copilot token
    let copilot_token = exchange_for_copilot_token(&github_token).await?;

    // Cache it
    save_copilot_token(&copilot_token)?;

    Ok(copilot_token)
}

// =============================================================================
// HELPERS
// =============================================================================

/// Get current Unix timestamp in seconds
fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Parse API base URL from Copilot token
///
/// Copilot tokens contain semicolon-delimited metadata like:
/// `token_value;proxy-ep=https://proxy.individual.githubcopilot.com;...`
///
/// We extract the `proxy-ep` and convert `proxy.` to `api.` in the hostname.
fn parse_api_base_from_token(token: &str) -> Option<String> {
    // Token format: "actual_token;key1=val1;key2=val2;..."
    // Look for account type from proxy-ep (e.g., proxy.individual.githubcopilot.com)
    for part in token.split(';') {
        if let Some(proxy_url) = part.strip_prefix("proxy-ep=") {
            // The proxy-ep tells us the account type (individual, business, etc.)
            // But the actual API URL is always https://api.githubcopilot.com for individuals
            // or https://api.{type}.githubcopilot.com for others
            if proxy_url.contains(".individual.") {
                return Some("https://api.githubcopilot.com".to_string());
            } else if let Some(account_type) = proxy_url
                .strip_prefix("proxy.")
                .and_then(|s| s.split('.').next())
            {
                return Some(format!("https://api.{}.githubcopilot.com", account_type));
            }
        }
    }

    None
}

// =============================================================================
// TESTS
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_api_base_from_token() {
        let token = "some_token_value;proxy-ep=proxy.individual.githubcopilot.com;other=data";
        let api_base = parse_api_base_from_token(token);
        assert_eq!(api_base, Some("https://api.githubcopilot.com".to_string()));
    }

    #[test]
    fn test_parse_api_base_no_proxy() {
        let token = "some_token_value;other=data";
        let api_base = parse_api_base_from_token(token);
        assert_eq!(api_base, None);
    }

    #[test]
    fn test_copilot_token_expiry() {
        let expired = CopilotToken {
            token: "test".to_string(),
            expires_at: current_timestamp() - 1000, // 1000s ago
            api_base: DEFAULT_COPILOT_API_BASE.to_string(),
        };
        assert!(expired.is_expired());

        let valid = CopilotToken {
            token: "test".to_string(),
            expires_at: current_timestamp() + 3600, // 1 hour from now
            api_base: DEFAULT_COPILOT_API_BASE.to_string(),
        };
        assert!(!valid.is_expired());
    }

    #[test]
    fn test_paths() {
        let github_path = get_github_token_path();
        assert!(github_path.ends_with("github_token.json"));

        let copilot_path = get_copilot_token_path();
        assert!(copilot_path.ends_with("copilot_token.json"));
    }
}
