use crate::polymarket::Trade;
use std::collections::HashSet;
use std::sync::Arc;
use teloxide::prelude::*;
use teloxide::types::ParseMode;
use tokio::sync::{mpsc, RwLock};

pub struct Notifier {
    bot: Bot,
    registered_chats: Arc<RwLock<HashSet<i64>>>,
    rx: mpsc::Receiver<Trade>,
}

impl Notifier {
    pub fn new(
        bot: Bot,
        registered_chats: Arc<RwLock<HashSet<i64>>>,
        rx: mpsc::Receiver<Trade>,
    ) -> Self {
        Self {
            bot,
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
    let addr_short = if trade.proxy_wallet.len() > 10 {
        format!(
            "{}...{}",
            &trade.proxy_wallet[..6],
            &trade.proxy_wallet[trade.proxy_wallet.len() - 4..]
        )
    } else {
        trade.proxy_wallet.clone()
    };

    let usdc_value = trade.usdc_value();
    let ts = chrono::DateTime::from_timestamp(trade.timestamp, 0)
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| trade.timestamp.to_string());

    format!(
        "🔔 <b>Trade Alert</b>\n\
         👛 Wallet: <code>{addr_short}</code>\n\
         📊 {side} {outcome}\n\
         📈 Market: {title}\n\
         💰 Price: ${price:.4} | Size: {size:.2} tokens (~${usdc:.2})\n\
         🕐 {ts}\n\
         🔗 <a href=\"https://polygonscan.com/tx/{tx_hash}\">View Tx</a>",
        addr_short = addr_short,
        side = trade.side,
        outcome = trade.outcome,
        title = trade.title,
        price = trade.price,
        size = trade.size,
        usdc = usdc_value,
        ts = ts,
        tx_hash = trade.transaction_hash,
    )
}
