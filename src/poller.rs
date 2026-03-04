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
    client: PolymarketClient,
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
        client: PolymarketClient,
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
            .map(|wallet| self.poll_wallet(wallet.proxy_wallet, wallet.alias))
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
        _alias: Option<String>,
    ) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let trades = match self.client.fetch_trades(&proxy_wallet).await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(proxy_wallet = %proxy_wallet, error = %e, "failed to fetch trades, skipping");
                return Ok(0);
            }
        };

        let seen = self.seen_tx_hashes.read().await;
        let new_trades = filter_new_trades(&trades, self.startup_timestamp, &seen);
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

        // Mark as seen and enqueue notifications
        let mut seen = self.seen_tx_hashes.write().await;
        for trade in new_trades {
            seen.insert(trade.transaction_hash.clone());
            if let Err(e) = self.notifier_tx.send(trade).await {
                tracing::error!(error = %e, "failed to enqueue trade notification");
            }
        }

        Ok(count)
    }
}
