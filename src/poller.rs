use crate::db;
use crate::filter::filter_new_trades;
use crate::polymarket::{PolymarketClient, Trade};
use futures::stream::{self, StreamExt};
use sqlx::PgPool;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, RwLock};
use tokio_util::sync::CancellationToken;

pub struct Poller {
    pool: PgPool,
    client: Arc<PolymarketClient>,
    notifier_tx: mpsc::Sender<Trade>,
    max_concurrency: usize,
    poll_interval: Duration,
    startup_timestamp: i64,
    seen_tx_hashes: Arc<RwLock<HashSet<String>>>,
    last_poll: Arc<RwLock<Option<std::time::Instant>>>,
}

impl Poller {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        pool: PgPool,
        client: Arc<PolymarketClient>,
        notifier_tx: mpsc::Sender<Trade>,
        max_concurrency: usize,
        poll_interval: Duration,
        startup_timestamp: i64,
        seen_tx_hashes: Arc<RwLock<HashSet<String>>>,
        last_poll: Arc<RwLock<Option<std::time::Instant>>>,
    ) -> Self {
        Self {
            pool,
            client,
            notifier_tx,
            max_concurrency,
            poll_interval,
            startup_timestamp,
            seen_tx_hashes,
            last_poll,
        }
    }

    pub async fn run(self, shutdown: CancellationToken) {
        tracing::info!(
            poll_interval_secs = self.poll_interval.as_secs(),
            max_concurrency = self.max_concurrency,
            "poller started"
        );

        loop {
            if shutdown.is_cancelled() {
                tracing::info!("poller shutting down");
                return;
            }

            if let Err(e) = self.poll_cycle().await {
                tracing::error!(error = %e, "poll cycle failed");
            }

            // Update last poll time
            {
                let mut lp = self.last_poll.write().await;
                *lp = Some(std::time::Instant::now());
            }

            tokio::select! {
                _ = shutdown.cancelled() => {
                    tracing::info!("poller shutting down");
                    return;
                }
                _ = tokio::time::sleep(self.poll_interval) => {}
            }
        }
    }

    async fn poll_cycle(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let wallets = db::list_wallets(&self.pool).await?;

        if wallets.is_empty() {
            tracing::debug!("no wallets to poll");
            return Ok(());
        }

        tracing::debug!(wallet_count = wallets.len(), "polling wallets");

        let results: Vec<_> = stream::iter(wallets)
            .map(|wallet| self.poll_wallet(wallet.proxy_wallet, wallet.alias, wallet.last_synced_timestamp))
            .buffer_unordered(self.max_concurrency)
            .collect()
            .await;

        let mut total_new = 0usize;
        for count in results.into_iter().flatten() {
            total_new += count;
        }

        if total_new > 0 {
            tracing::info!(new_trades = total_new, "poll cycle complete");
        }

        Ok(())
    }

    async fn poll_wallet(
        &self,
        proxy_wallet: String,
        alias: Option<String>,
        last_synced_timestamp: Option<i64>,
    ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let since_ts = last_synced_timestamp.unwrap_or(self.startup_timestamp);

        let trades = match self.client.fetch_trades_since(&proxy_wallet, since_ts).await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(proxy_wallet = %proxy_wallet, error = %e, "failed to fetch trades, skipping");
                return Ok(0);
            }
        };

        let seen = self.seen_tx_hashes.read().await;
        let new_trades = filter_new_trades(&trades, since_ts, &seen);
        drop(seen);

        let count = new_trades.len();
        if count == 0 {
            return Ok(0);
        }

        tracing::info!(
            proxy_wallet = %proxy_wallet,
            new_trades = count,
            "found new trades"
        );

        // Update last_synced_timestamp to the newest trade we found
        let max_ts = new_trades.iter().map(|t| t.timestamp).max().unwrap_or(since_ts);
        if let Err(e) = db::update_last_synced_timestamp(&self.pool, &proxy_wallet, max_ts).await {
            tracing::warn!(error = %e, "failed to update last_synced_timestamp");
        }

        // Mark as seen and enqueue notifications
        let mut seen = self.seen_tx_hashes.write().await;
        for mut trade in new_trades {
            trade.alias = alias.clone();
            seen.insert(trade.transaction_hash.clone());
            if let Err(e) = self.notifier_tx.send(trade).await {
                tracing::error!(error = %e, "failed to enqueue trade notification");
            }
        }

        Ok(count)
    }
}
