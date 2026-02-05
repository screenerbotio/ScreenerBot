# GitHub Copilot Authentication Module

This module provides OAuth Device Code Flow authentication for GitHub Copilot API.

## Features

- ✅ OAuth Device Code Flow (user-friendly, no browser redirect)
- ✅ GitHub access token management
- ✅ Copilot API token exchange
- ✅ Token caching and auto-refresh
- ✅ Automatic API endpoint detection

## Usage

### Complete OAuth Flow

```rust
use screenerbot::apis::llm::copilot::auth;

// Step 1: Request device code
let device_code = auth::request_device_code().await?;

println!("Go to: {}", device_code.verification_uri);
println!("Enter code: {}", device_code.user_code);

// Step 2: Poll for authorization (loop with interval)
let mut github_token = None;
let interval = std::time::Duration::from_secs(device_code.interval);

while github_token.is_none() {
    tokio::time::sleep(interval).await;
    
    match auth::poll_for_access_token(&device_code.device_code).await {
        Ok(Some(token)) => {
            github_token = Some(token);
            break;
        }
        Ok(None) => {
            // Still waiting...
            continue;
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            break;
        }
    }
}

let github_token = github_token.expect("Authorization failed");

// Step 3: Save GitHub token
auth::save_github_token(&github_token)?;

// Step 4: Exchange for Copilot token
let copilot_token = auth::exchange_for_copilot_token(&github_token).await?;

// Step 5: Cache Copilot token
auth::save_copilot_token(&copilot_token)?;

println!("Authenticated! Token expires at: {}", copilot_token.expires_at);
```

### Simple Usage (Auto-Refresh)

Once authenticated, just use:

```rust
use screenerbot::apis::llm::copilot::auth;

// Gets cached token or auto-refreshes if expired
let token = auth::get_valid_copilot_token().await?;

// Use token with Copilot API
println!("API Base: {}", token.api_base);
println!("Token: {}", token.token);
```

### Check Token Status

```rust
use screenerbot::apis::llm::copilot::auth;

// Check if GitHub token exists
if let Some(github_token) = auth::load_github_token() {
    println!("GitHub token: {}", github_token);
}

// Check if Copilot token is cached and valid
if let Some(copilot_token) = auth::load_copilot_token() {
    println!("Copilot token valid until: {}", copilot_token.expires_at);
} else {
    println!("No valid Copilot token cached");
}
```

## Token Storage

Tokens are stored in the app data directory:

- **GitHub token**: `~/Library/Application Support/ScreenerBot/data/github_token.json`
- **Copilot token**: `~/Library/Application Support/ScreenerBot/data/copilot_token.json`

## API Endpoints

The module uses these GitHub endpoints:

1. **Device Code**: `https://github.com/login/device/code`
2. **Access Token**: `https://github.com/login/oauth/access_token`
3. **Copilot Token**: `https://api.github.com/copilot_internal/v2/token`

The Copilot API base URL is extracted from the token response (usually `https://api.individual.githubcopilot.com`).

## Error Handling

All functions return `Result<T, String>` for simplicity:

```rust
match auth::get_valid_copilot_token().await {
    Ok(token) => {
        // Use token
    }
    Err(e) if e.contains("No GitHub token") => {
        // Need to run OAuth flow first
    }
    Err(e) => {
        // Other error
        eprintln!("Error: {}", e);
    }
}
```

## Token Expiry

Copilot tokens typically expire after 30 minutes. The module:

- Checks expiry with a 5-minute buffer
- Auto-refreshes when calling `get_valid_copilot_token()`
- Uses the cached GitHub token for refresh

## Testing

Run the tests:

```bash
cargo test --lib copilot::auth::tests
```

Available tests:

- `test_parse_api_base_from_token` - Token parsing logic
- `test_parse_api_base_no_proxy` - Fallback behavior
- `test_copilot_token_expiry` - Expiry checking
- `test_paths` - File path generation
