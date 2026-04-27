use anyhow::{Context, Result};
use serde::Deserialize;

/// SiliconFlow balance response structure.
/// API: GET https://api.siliconflow.cn/v1/user/info
/// Auth: Authorization: Bearer <api_key>
#[derive(Clone, Debug, Deserialize)]
pub struct SiliconFlowBalanceResponse {
    pub data: SiliconFlowBalanceData,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SiliconFlowBalanceData {
    #[serde(rename = "totalBalance")]
    pub total_balance: String,
}

/// Query SiliconFlow account balance.
///
/// # Arguments
/// * `http` - reqwest HTTP client
/// * `api_key` - SiliconFlow API key
pub async fn query_siliconflow_balance(
    http: &reqwest::Client,
    api_key: &str,
) -> Result<SiliconFlowBalanceResponse> {
    let url = "https://api.siliconflow.cn/v1/user/info";

    let response = http
        .get(url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Accept", "application/json")
        .send()
        .await
        .context("failed to query SiliconFlow balance")?;

    let status = response.status();
    if !status.is_success() {
        anyhow::bail!(
            "SiliconFlow balance API returned {}: {}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("unknown")
        );
    }

    let balance = response
        .json::<SiliconFlowBalanceResponse>()
        .await
        .context("failed to parse SiliconFlow balance response")?;

    Ok(balance)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_deserialize_response() {
        let json = r#"{
            "data": {
                "totalBalance": "100.00"
            }
        }"#;

        let response: SiliconFlowBalanceResponse = serde_json::from_str(json).unwrap();

        assert_eq!(response.data.total_balance, "100.00");
    }
}
