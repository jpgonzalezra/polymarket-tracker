use crate::db;
use crate::polymarket::Trade;
use sqlx::PgPool;
use std::collections::HashSet;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::{LinkPreviewOptions, ParseMode};
use tokio::sync::{mpsc, RwLock};

pub struct Notifier {
    bot: Bot,
    pool: PgPool,
    registered_chats: Arc<RwLock<HashSet<i64>>>,
    rx: mpsc::Receiver<Trade>,
}

impl Notifier {
    pub fn new(
        bot: Bot,
        pool: PgPool,
        registered_chats: Arc<RwLock<HashSet<i64>>>,
        rx: mpsc::Receiver<Trade>,
    ) -> Self {
        Self {
            bot,
            pool,
            registered_chats,
            rx,
        }
    }

    pub async fn run(mut self, shutdown: tokio_util::sync::CancellationToken) {
        tracing::info!("notifier started");
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => {
                    while let Ok(trade) = self.rx.try_recv() {
                        if let Err(e) = self.send_trade_alert(&trade).await {
                            tracing::error!(error = %e, "failed to send trade alert during shutdown");
                        }
                    }
                    tracing::info!("notifier shutting down");
                    return;
                }
                msg = self.rx.recv() => {
                    match msg {
                        Some(trade) => {
                            if let Err(e) = self.send_trade_alert(&trade).await {
                                tracing::error!(
                                    error = %e,
                                    tx_hash = %trade.transaction_hash,
                                    "failed to send trade alert"
                                );
                            }
                            // Rate limit: 1 msg/sec
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        }
                        None => {
                            tracing::info!("notifier channel closed");
                            return;
                        }
                    }
                }
            }
        }
    }

    async fn send_trade_alert(&self, trade: &Trade) -> Result<(), teloxide::RequestError> {
        let still_watched = db::wallet_exists(&self.pool, &trade.proxy_wallet)
            .await
            .unwrap_or(true);
        if !still_watched {
            tracing::info!(
                proxy_wallet = %trade.proxy_wallet,
                tx_hash = %trade.transaction_hash,
                "wallet removed, skipping queued alert"
            );
            return Ok(());
        }

        let message = format_trade_message(trade);
        let chats: Vec<i64> = self.registered_chats.read().await.iter().copied().collect();

        if chats.is_empty() {
            tracing::warn!(
                tx_hash = %trade.transaction_hash,
                "no registered chats — dropping alert (send any command to the bot first)"
            );
            return Ok(());
        }

        for chat_id in chats {
            if let Err(e) = self
                .bot
                .send_message(ChatId(chat_id), &message)
                .parse_mode(ParseMode::Html)
                .link_preview_options(LinkPreviewOptions {
                    is_disabled: true,
                    url: None,
                    prefer_small_media: false,
                    prefer_large_media: false,
                    show_above_text: false,
                })
                .send()
                .await
            {
                tracing::error!(chat_id, error = %e, "failed to send to chat");
            }
        }

        tracing::info!(
            tx_hash = %trade.transaction_hash,
            proxy_wallet = %trade.proxy_wallet,
            side = %trade.side,
            "trade alert sent"
        );
        Ok(())
    }
}

fn format_trade_message(trade: &Trade) -> String {
    let addr = &trade.proxy_wallet;
    let addr_short = if addr.len() > 10 {
        format!("{}...{}", &addr[..6], &addr[addr.len() - 4..])
    } else {
        addr.clone()
    };

    let alias_str = trade
        .alias
        .as_ref()
        .map(|a| format!(" ({})", a))
        .unwrap_or_default();

    let wallet_line = format!(
        "👛 <a href=\"https://polymarketanalytics.com/traders/{addr}\">{addr_short}</a>{alias_str}",
        addr = addr,
        addr_short = addr_short,
        alias_str = alias_str,
    );

    let market_slug = trade
        .event_slug
        .as_deref()
        .unwrap_or(trade.slug.as_str());

    let market_line = format!(
        "📈 <a href=\"https://polymarket.com/event/{slug}\">{title}</a>",
        slug = market_slug,
        title = trade.title,
    );

    let usdc_value = trade.usdc_value();
    let ts = chrono::DateTime::from_timestamp(trade.timestamp, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| trade.timestamp.to_string());

    format!(
        "🔔 <b>Trade Alert</b>\n\
         {wallet_line}\n\
         📊 {side} {outcome}\n\
         {market_line}\n\
         💰 Price: ${price:.4} | Size: {size:.2} tokens (~${usdc:.2})\n\
         🕐 {ts}\n\
         🔗 <a href=\"https://polygonscan.com/tx/{tx_hash}\">View Tx</a>",
        wallet_line = wallet_line,
        side = trade.side,
        outcome = trade.outcome,
        market_line = market_line,
        price = trade.price,
        size = trade.size,
        usdc = usdc_value,
        ts = ts,
        tx_hash = trade.transaction_hash,
    )
}
