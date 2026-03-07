use crate::bot_score::{format_bot_score, BotScorePipeline};
use crate::db;
use crate::db::TradeFilters;
use crate::polymarket::PolymarketClient;
use sqlx::PgPool;
use std::sync::Arc;
use teloxide::macros::BotCommands;
use teloxide::prelude::*;
use teloxide::types::ParseMode;
use teloxide::utils::command::BotCommands as BotCommandsTrait;
use tokio::sync::RwLock;

#[derive(BotCommands, Clone, Debug)]
#[command(rename_rule = "lowercase", description = "Available commands:")]
pub enum Command {
    #[command(description = "Show this help message")]
    Help,
    #[command(description = "Add wallet: /add &lt;0xAddress&gt; [alias]")]
    Add(String),
    #[command(description = "Remove wallet: /remove &lt;0xAddress&gt;")]
    Remove(String),
    #[command(description = "List watched wallets")]
    List,
    #[command(description = "Show bot status")]
    Status,
    #[command(description = "Subscribe this chat to trade alerts (admin only)")]
    Subscribe,
    #[command(description = "Analyze wallet bot score: /botscore &lt;0xAddress&gt;")]
    Botscore(String),
    #[command(description = "Set filter: /setfilter amount|liquidity [value]")]
    SetFilter(String),
    #[command(description = "Remove filter: /removefilter amount|liquidity")]
    RemoveFilter(String),
    #[command(description = "Show active trade filters")]
    Filters,
}

#[derive(Clone)]
pub struct BotState {
    pub pool: PgPool,
    pub admin_user_ids: Vec<u64>,
    pub start_time: std::time::Instant,
    pub last_poll: Arc<RwLock<Option<std::time::Instant>>>,
    pub registered_chats: Arc<RwLock<std::collections::HashSet<i64>>>,
    pub api_client: Arc<PolymarketClient>,
    pub trade_filters: Arc<RwLock<TradeFilters>>,
}

fn is_admin(user_id: UserId, admin_ids: &[u64]) -> bool {
    admin_ids.contains(&user_id.0)
}

fn parse_wallet_address(s: &str) -> Result<String, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("Address is required.".to_string());
    }
    if !s.starts_with("0x") || s.len() != 42 {
        return Err("Must start with 0x and be 42 characters.".to_string());
    }
    if !s[2..].chars().all(|c| c.is_ascii_hexdigit()) {
        return Err("Must contain only hex characters.".to_string());
    }
    Ok(s.to_string())
}

fn parse_add_args(args: &str) -> Result<(String, Option<String>), String> {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.is_empty() {
        return Err("Usage: /add <0xProxyWallet> [alias]".to_string());
    }
    let address = parse_wallet_address(parts[0])?;
    let alias = parts.get(1).map(|s| s.to_string());
    Ok((address, alias))
}

fn parse_remove_args(args: &str) -> Result<String, String> {
    let address = args.trim();
    if address.is_empty() {
        return Err("Usage: /remove <0xProxyWallet>".to_string());
    }
    parse_wallet_address(address)
}

fn parse_filter_key(input: &str) -> Result<&'static str, String> {
    match input.trim().to_lowercase().as_str() {
        "amount" | "min_amount" => Ok("min_amount"),
        "liquidity" | "min_liquidity" => Ok("min_liquidity"),
        "" => Err("Usage: /removefilter amount|liquidity".to_string()),
        other => Err(format!(
            "Unknown filter '{}'. Valid filters: amount, liquidity",
            other
        )),
    }
}

fn parse_set_filter_args(args: &str) -> Result<(&'static str, f64), String> {
    let parts: Vec<&str> = args.split_whitespace().collect();
    if parts.len() != 2 {
        return Err("Usage: /setfilter amount|liquidity [value]".to_string());
    }
    let key = parse_filter_key(parts[0])?;
    let value: f64 = parts[1]
        .parse()
        .map_err(|_| "Value must be a number.".to_string())?;
    if value < 0.0 {
        return Err("Value must be non-negative.".to_string());
    }
    Ok((key, value))
}

fn filter_key_label(key: &str) -> &str {
    match key {
        "min_amount" => "Min trade amount",
        "min_liquidity" => "Min market liquidity",
        _ => key,
    }
}

pub async fn handle_command(
    bot: Bot,
    msg: Message,
    cmd: Command,
    state: BotState,
) -> Result<(), teloxide::RequestError> {
    let response = match cmd {
        Command::Help => Command::descriptions().to_string(),

        Command::Add(args) => {
            let user_id = msg.from.as_ref().map(|u| u.id).unwrap_or(UserId(0));
            if !is_admin(user_id, &state.admin_user_ids) {
                "⛔ You are not authorized to add wallets.".to_string()
            } else {
                match parse_add_args(&args) {
                    Ok((address, alias)) => {
                        match db::add_wallet(&state.pool, &address, alias.as_deref()).await {
                            Ok(()) => {
                                let alias_str = alias
                                    .map(|a| format!(" ({})", a))
                                    .unwrap_or_default();
                                format!("✅ Added wallet {}{}", &address[..10], alias_str)
                            }
                            Err(e) => format!("❌ Database error: {}", e),
                        }
                    }
                    Err(e) => format!("❌ {}", e),
                }
            }
        }

        Command::Remove(args) => {
            let user_id = msg.from.as_ref().map(|u| u.id).unwrap_or(UserId(0));
            if !is_admin(user_id, &state.admin_user_ids) {
                "⛔ You are not authorized to remove wallets.".to_string()
            } else {
                match parse_remove_args(&args) {
                    Ok(address) => match db::remove_wallet(&state.pool, &address).await {
                        Ok(true) => format!("✅ Removed wallet {}", &address[..10]),
                        Ok(false) => "⚠️ Wallet not found in watchlist.".to_string(),
                        Err(e) => format!("❌ Database error: {}", e),
                    },
                    Err(e) => format!("❌ {}", e),
                }
            }
        }

        Command::List => match db::list_wallets(&state.pool).await {
            Ok(wallets) => {
                if wallets.is_empty() {
                    "📋 No wallets being watched.".to_string()
                } else {
                    let mut lines = vec!["📋 Watched wallets:".to_string()];
                    for w in &wallets {
                        let alias_str = w
                            .alias
                            .as_ref()
                            .map(|a| format!(" ({})", a))
                            .unwrap_or_default();
                        lines.push(format!("  • {}{}", w.proxy_wallet, alias_str));
                    }
                    lines.join("\n")
                }
            }
            Err(e) => format!("❌ Database error: {}", e),
        },

        Command::Status => {
            let uptime = state.start_time.elapsed();
            let hours = uptime.as_secs() / 3600;
            let mins = (uptime.as_secs() % 3600) / 60;

            let wallet_count = db::list_wallets(&state.pool)
                .await
                .map(|w| w.len())
                .unwrap_or(0);

            let last_poll_str = {
                let lp = state.last_poll.read().await;
                match *lp {
                    Some(t) => format!("{}s ago", t.elapsed().as_secs()),
                    None => "never".to_string(),
                }
            };

            format!(
                "📊 Status\n\
                 ⏱ Uptime: {}h {}m\n\
                 👛 Wallets watched: {}\n\
                 🔄 Last poll: {}",
                hours, mins, wallet_count, last_poll_str
            )
        }

        Command::Subscribe => {
            let user_id = msg.from.as_ref().map(|u| u.id).unwrap_or(UserId(0));
            tracing::info!(user_id = user_id.0, "user attempting to subscribe");
            if !is_admin(user_id, &state.admin_user_ids) {
                "⛔ You are not authorized to subscribe. Only admins can receive trade alerts."
                    .to_string()
            } else {
                let chat_id = msg.chat.id.0;
                match db::insert_registered_chat(&state.pool, chat_id).await {
                    Ok(()) => {
                        tracing::info!(user_id = user_id.0, chat_id, "chat subscribed successfully");
                        state.registered_chats.write().await.insert(chat_id);
                        "✅ This chat is now subscribed to trade alerts.".to_string()
                    }
                    Err(e) => {
                        tracing::error!(chat_id, error = %e, "failed to persist chat subscription");
                        format!("❌ Database error: {}", e)
                    }
                }
            }
        }

        Command::Botscore(args) => {
            let user_id = msg.from.as_ref().map(|u| u.id).unwrap_or(UserId(0));
            tracing::info!(user_id = user_id.0, wallet = %args, "botscore command received");
            if !is_admin(user_id, &state.admin_user_ids) {
                "⛔ You are not authorized to use this command.".to_string()
            } else {
                match parse_wallet_address(&args) {
                    Err(e) => {
                        tracing::warn!(error = %e, "invalid wallet address");
                        format!("❌ {}", e)
                    }
                    Ok(address) => {
                        tracing::info!(wallet = %address, "fetching bot score");
                        let since = chrono::Utc::now().timestamp() - 7 * 86_400;
                        let alias = db::get_wallet_alias(&state.pool, &address).await.ok().flatten();
                        let (all_result, taker_result) = tokio::join!(
                            state.api_client.fetch_trades_since(&address, since),
                            state.api_client.fetch_taker_trades_since(&address, since),
                        );
                        match (all_result, taker_result) {
                            (Ok(all), Ok(taker)) => {
                                if all.is_empty() {
                                    tracing::info!(wallet = %address, "no trades found");
                                    format!(
                                        "No trades found for {} in the last 7 days.",
                                        &address[..10]
                                    )
                                } else {
                                    tracing::info!(wallet = %address, trades = all.len(), "computing bot score");
                                    let result =
                                        BotScorePipeline::default().run(&all, &taker);
                                    format_bot_score(&address, alias.as_deref(), &result)
                                }
                            }
                            (all_err, taker_err) => {
                                tracing::error!(wallet = %address, all_error = ?all_err, taker_error = ?taker_err, "API error fetching trades");
                                "❌ API error fetching trade data.".to_string()
                            }
                        }
                    }
                }
            }
        }

        Command::SetFilter(args) => {
            let user_id = msg.from.as_ref().map(|u| u.id).unwrap_or(UserId(0));
            if !is_admin(user_id, &state.admin_user_ids) {
                "⛔ You are not authorized to set filters.".to_string()
            } else {
                match parse_set_filter_args(&args) {
                    Ok((key, value)) => {
                        match db::set_trade_filter(&state.pool, key, value).await {
                            Ok(()) => {
                                let mut filters = state.trade_filters.write().await;
                                match key {
                                    "min_amount" => filters.min_amount = Some(value),
                                    "min_liquidity" => filters.min_liquidity = Some(value),
                                    _ => {}
                                }
                                let label = filter_key_label(key);
                                format!("✅ Filter set: {} >= ${:.2}", label, value)
                            }
                            Err(e) => format!("❌ Database error: {}", e),
                        }
                    }
                    Err(e) => format!("❌ {}", e),
                }
            }
        }

        Command::RemoveFilter(args) => {
            let user_id = msg.from.as_ref().map(|u| u.id).unwrap_or(UserId(0));
            if !is_admin(user_id, &state.admin_user_ids) {
                "⛔ You are not authorized to remove filters.".to_string()
            } else {
                match parse_filter_key(&args) {
                    Ok(key) => match db::remove_trade_filter(&state.pool, key).await {
                        Ok(true) => {
                            let mut filters = state.trade_filters.write().await;
                            match key {
                                "min_amount" => filters.min_amount = None,
                                "min_liquidity" => filters.min_liquidity = None,
                                _ => {}
                            }
                            let label = filter_key_label(key);
                            format!("✅ Filter removed: {}", label)
                        }
                        Ok(false) => "⚠️ No such filter is active.".to_string(),
                        Err(e) => format!("❌ Database error: {}", e),
                    },
                    Err(e) => format!("❌ {}", e),
                }
            }
        }

        Command::Filters => {
            let filters = state.trade_filters.read().await;
            let mut lines = vec!["🔍 Active filters:".to_string()];
            let mut has_filter = false;
            if let Some(v) = filters.min_amount {
                lines.push(format!("  • Min trade amount: ${:.2} USDC", v));
                has_filter = true;
            }
            if let Some(v) = filters.min_liquidity {
                lines.push(format!("  • Min market liquidity: ${:.2}", v));
                has_filter = true;
            }
            if !has_filter {
                "🔍 No active filters. Use /setfilter to add one.".to_string()
            } else {
                lines.join("\n")
            }
        }
    };

    bot.send_message(msg.chat.id, response)
        .parse_mode(ParseMode::Html)
        .send()
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_add_with_alias() {
        let (addr, alias) = parse_add_args("0x1234567890abcdef1234567890abcdef12345678 whale1").unwrap();
        assert_eq!(addr, "0x1234567890abcdef1234567890abcdef12345678");
        assert_eq!(alias, Some("whale1".to_string()));
    }

    #[test]
    fn test_parse_add_without_alias() {
        let (addr, alias) = parse_add_args("0x1234567890abcdef1234567890abcdef12345678").unwrap();
        assert_eq!(addr, "0x1234567890abcdef1234567890abcdef12345678");
        assert_eq!(alias, None);
    }

    #[test]
    fn test_parse_add_invalid_address() {
        let result = parse_add_args("invalid_address");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_add_short_address() {
        let result = parse_add_args("0x1234");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_add_empty() {
        let result = parse_add_args("");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_remove_valid() {
        let addr = parse_remove_args("0x1234567890abcdef1234567890abcdef12345678").unwrap();
        assert_eq!(addr, "0x1234567890abcdef1234567890abcdef12345678");
    }

    #[test]
    fn test_parse_remove_empty() {
        let result = parse_remove_args("");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_remove_invalid() {
        let result = parse_remove_args("not-an-address");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_set_filter_amount() {
        let (key, value) = parse_set_filter_args("amount 50").unwrap();
        assert_eq!(key, "min_amount");
        assert!((value - 50.0).abs() < 0.001);
    }

    #[test]
    fn test_parse_set_filter_liquidity() {
        let (key, value) = parse_set_filter_args("liquidity 1000.5").unwrap();
        assert_eq!(key, "min_liquidity");
        assert!((value - 1000.5).abs() < 0.001);
    }

    #[test]
    fn test_parse_set_filter_missing_value() {
        let result = parse_set_filter_args("amount");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_set_filter_invalid_key() {
        let result = parse_set_filter_args("unknown 50");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_set_filter_negative() {
        let result = parse_set_filter_args("amount -10");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_filter_key_aliases() {
        assert_eq!(parse_filter_key("amount").unwrap(), "min_amount");
        assert_eq!(parse_filter_key("min_amount").unwrap(), "min_amount");
        assert_eq!(parse_filter_key("liquidity").unwrap(), "min_liquidity");
        assert_eq!(parse_filter_key("min_liquidity").unwrap(), "min_liquidity");
    }

    #[test]
    fn test_is_admin() {
        assert!(is_admin(UserId(123), &[123, 456]));
        assert!(!is_admin(UserId(789), &[123, 456]));
    }
}
