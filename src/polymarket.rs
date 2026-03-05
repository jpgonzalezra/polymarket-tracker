use rand::Rng;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::debug;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Trade {
    pub proxy_wallet: String,
    pub side: String,
    pub size: f64,
    pub price: f64,
    pub timestamp: i64,
    pub title: String,
    pub outcome: String,
    pub outcome_index: i32,
    pub transaction_hash: String,
    pub condition_id: String,
    pub slug: String,
    #[serde(default)]
    pub event_slug: Option<String>,
    #[serde(default)]
    pub asset: Option<String>,
    #[serde(skip)]
    pub alias: Option<String>,
}

impl Trade {
    pub fn usdc_value(&self) -> f64 {
        self.size * self.price
    }
}

pub struct PolymarketClient {
    client: reqwest::Client,
    base_url: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("max retries exceeded")]
    MaxRetries,
}

impl PolymarketClient {
    pub fn new(base_url: &str) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent("polymarket-tracker/0.1")
            .build()
            .expect("failed to build HTTP client");

        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    /// Fetches only trades newer than `since_timestamp` (all fills).
    pub async fn fetch_trades_since(
        &self,
        proxy_wallet: &str,
        since_timestamp: i64,
    ) -> Result<Vec<Trade>, ApiError> {
        self.fetch_trades_since_impl(proxy_wallet, since_timestamp, false)
            .await
    }

    /// Fetches only taker trades newer than `since_timestamp`.
    pub async fn fetch_taker_trades_since(
        &self,
        proxy_wallet: &str,
        since_timestamp: i64,
    ) -> Result<Vec<Trade>, ApiError> {
        self.fetch_trades_since_impl(proxy_wallet, since_timestamp, true)
            .await
    }

    async fn fetch_trades_since_impl(
        &self,
        proxy_wallet: &str,
        since_timestamp: i64,
        taker_only: bool,
    ) -> Result<Vec<Trade>, ApiError> {
        let mut new_trades = Vec::new();
        let mut offset = 0u32;
        let limit = 100u32;

        loop {
            let page = self
                .fetch_trades_page(proxy_wallet, limit, offset, taker_only)
                .await?;
            let page_len = page.len();

            let mut done = page_len < limit as usize;
            for trade in page {
                if trade.timestamp <= since_timestamp {
                    done = true;
                } else {
                    new_trades.push(trade);
                }
            }

            if done {
                break;
            }

            offset += limit;
            if offset >= 10_000 {
                break;
            }
        }

        Ok(new_trades)
    }

    async fn fetch_trades_page(
        &self,
        proxy_wallet: &str,
        limit: u32,
        offset: u32,
        taker_only: bool,
    ) -> Result<Vec<Trade>, ApiError> {
        let url = format!("{}/trades", self.base_url);
        let taker_only_str = if taker_only { "true" } else { "false" };
        let full_url = format!(
            "{}?user={}&limit={}&offset={}&takerOnly={}",
            url, proxy_wallet, limit, offset, taker_only_str
        );
        debug!(url = %full_url, "consuming polymarket API");

        for attempt in 0..4u32 {
            let result = self
                .client
                .get(&url)
                .query(&[
                    ("user", proxy_wallet),
                    ("limit", &limit.to_string()),
                    ("offset", &offset.to_string()),
                    ("takerOnly", taker_only_str),
                ])
                .send()
                .await;

            match result {
                Ok(resp) => {
                    let status = resp.status();
                    if status.is_success() {
                        let trades: Vec<Trade> = resp.json().await?;
                        return Ok(trades);
                    }
                    if status == 429 || status.is_server_error() {
                        tracing::warn!(
                            status = %status,
                            attempt,
                            proxy_wallet,
                            "retryable API error, backing off"
                        );
                        Self::backoff(attempt).await;
                        continue;
                    }
                    // Non-retryable error
                    tracing::error!(status = %status, proxy_wallet, "non-retryable API error");
                    return Ok(vec![]);
                }
                Err(e) if e.is_timeout() || e.is_connect() => {
                    tracing::warn!(attempt, proxy_wallet, error = %e, "timeout/connect error, backing off");
                    Self::backoff(attempt).await;
                    continue;
                }
                Err(e) => return Err(ApiError::Http(e)),
            }
        }

        Err(ApiError::MaxRetries)
    }

    async fn backoff(attempt: u32) {
        let base_ms = 1000u64 * 2u64.pow(attempt); // 1s, 2s, 4s, 8s
        let jitter = rand::thread_rng().gen_range(0..500);
        tokio::time::sleep(Duration::from_millis(base_ms + jitter)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trade_deserialization() {
        let json = r#"[
            {
                "proxyWallet": "0x1234567890abcdef1234567890abcdef12345678",
                "side": "BUY",
                "size": 100.0,
                "price": 0.65,
                "timestamp": 1700000000,
                "title": "Will X happen?",
                "outcome": "Yes",
                "outcomeIndex": 0,
                "transactionHash": "0xabc123",
                "conditionId": "0xdef456",
                "slug": "will-x-happen",
                "eventSlug": "event-slug",
                "asset": "12345"
            }
        ]"#;

        let trades: Vec<Trade> = serde_json::from_str(json).unwrap();
        assert_eq!(trades.len(), 1);
        let t = &trades[0];
        assert_eq!(t.side, "BUY");
        assert_eq!(t.size, 100.0);
        assert_eq!(t.price, 0.65);
        assert_eq!(t.outcome, "Yes");
        assert_eq!(t.transaction_hash, "0xabc123");
        assert!((t.usdc_value() - 65.0).abs() < 0.001);
    }

    #[test]
    fn test_trade_optional_fields() {
        let json = r#"[
            {
                "proxyWallet": "0x1234",
                "side": "SELL",
                "size": 50.0,
                "price": 0.30,
                "timestamp": 1700000000,
                "title": "Market title",
                "outcome": "No",
                "outcomeIndex": 1,
                "transactionHash": "0xdef",
                "conditionId": "0xabc",
                "slug": "market-title"
            }
        ]"#;

        let trades: Vec<Trade> = serde_json::from_str(json).unwrap();
        assert_eq!(trades.len(), 1);
        assert!(trades[0].event_slug.is_none());
        assert!(trades[0].asset.is_none());
    }
}
