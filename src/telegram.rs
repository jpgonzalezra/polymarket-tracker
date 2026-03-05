use crate::bot_score::{format_bot_score, BotScorePipeline};
use crate::db;
use crate::polymarket::PolymarketClient;
use sqlx::PgPool;
use std::sync::Arc;
use teloxide::macros::BotCommands;
use teloxide::prelude::*;
use teloxide::utils::command::BotCommands as BotCommandsTrait;
use tokio::sync::RwLock;

#[derive(BotCommands, Clone, Debug)]
#[command(rename_rule = "lowercase", description = "Available commands:")]
pub enum Command {
    #[command(description = "Show this help message")]
    Help,
    #[command(description = "Add wallet: /add <0xAddress> [alias]")]
    Add(String),
    #[command(description = "Remove wallet: /remove <0xAddress>")]
    Remove(String),
    #[command(description = "List watched wallets")]
    List,
    #[command(description = "Show bot status")]
    Status,
    #[command(description = "Subscribe this chat to trade alerts (admin only)")]
    Subscribe,
    #[command(description = "Analyze wallet bot score: /botscore <0xAddress>")]
    Botscore(String),
}

#[derive(Clone)]
pub struct BotState {
    pub pool: PgPool,
    pub admin_user_ids: Vec<u64>,
    pub start_time: std::time::Instant,
    pub last_poll: Arc<RwLock<Option<std::time::Instant>>>,
    pub registered_chats: Arc<RwLock<std::collections::HashSet<i64>>>,
    pub api_client: Arc<PolymarketClient>,
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
            if !is_admin(user_id, &state.admin_user_ids) {
                "⛔ You are not authorized to use this command.".to_string()
            } else {
                match parse_wallet_address(&args) {
                    Err(e) => format!("❌ {}", e),
                    Ok(address) => {
                        let since = chrono::Utc::now().timestamp() - 7 * 86_400;
                        let (all_result, taker_result) = tokio::join!(
                            state.api_client.fetch_trades_since(&address, since),
                            state.api_client.fetch_taker_trades_since(&address, since),
                        );
                        match (all_result, taker_result) {
                            (Ok(all), Ok(taker)) => {
                                if all.is_empty() {
                                    format!(
                                        "No trades found for {} in the last 7 days.",
                                        &address[..10]
                                    )
                                } else {
                                    let result =
                                        BotScorePipeline::default().run(&all, &taker);
                                    format_bot_score(&address, &result)
                                }
                            }
                            _ => "❌ API error fetching trade data.".to_string(),
                        }
                    }
                }
            }
        }
    };

    bot.send_message(msg.chat.id, response).send().await?;
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
    fn test_is_admin() {
        assert!(is_admin(UserId(123), &[123, 456]));
        assert!(!is_admin(UserId(789), &[123, 456]));
    }
}
