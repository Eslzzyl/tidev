use anyhow::{Context, Result};
use serde::Deserialize;

/// DeepSeek balance response structure.
/// API: GET <https://api.deepseek.com/user/balance>
/// Auth: Authorization: Bearer <api_key>
#[derive(Clone, Debug, Deserialize)]
pub struct DeepSeekBalanceResponse {
    pub is_available: bool,
    pub balance_infos: Vec<DeepSeekBalanceInfo>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DeepSeekBalanceInfo {
    pub currency: String,
    pub total_balance: String,
    pub granted_balance: String,
    pub topped_up_balance: String,
}

/// Query DeepSeek account balance.
///
/// # Arguments
/// * `http` - reqwest HTTP client
/// * `api_key` - DeepSeek API key
pub async fn query_deepseek_balance(
    http: &reqwest::Client,
    api_key: &str,
) -> Result<DeepSeekBalanceResponse> {
    let url = "https://api.deepseek.com/user/balance";

    let response = http
        .get(url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Accept", "application/json")
        .send()
        .await
        .context("failed to query DeepSeek balance")?;

    let status = response.status();
    if !status.is_success() {
        anyhow::bail!(
            "DeepSeek balance API returned {}: {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("unknown")
        );
    }

    let balance = response
        .json::<DeepSeekBalanceResponse>()
        .await
        .context("failed to parse DeepSeek balance response")?;

    Ok(balance)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_response() {
        let json = r#"{
            "is_available": true,
            "balance_infos": [
                {
                    "currency": "CNY",
                    "total_balance": "110.00",
                    "granted_balance": "10.00",
                    "topped_up_balance": "100.00"
                }
            ]
        }"#;

        let response: DeepSeekBalanceResponse = serde_json::from_str(json).unwrap();

        assert!(response.is_available);
        assert_eq!(response.balance_infos.len(), 1);

        let info = &response.balance_infos[0];
        assert_eq!(info.currency, "CNY");
        assert_eq!(info.total_balance, "110.00");
        assert_eq!(info.granted_balance, "10.00");
        assert_eq!(info.topped_up_balance, "100.00");
    }
}
