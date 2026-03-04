use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    pub telegram_bot_token: String,
    pub admin_user_ids: Vec<u64>,
    pub database_url: String,
    pub polymarket_api_base_url: String,
    pub poll_interval_secs: u64,
    pub max_concurrency: usize,
    pub http_port: u16,
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        let telegram_bot_token =
            env::var("TELEGRAM_BOT_TOKEN").map_err(|_| ConfigError::Missing("TELEGRAM_BOT_TOKEN"))?;

        let admin_user_ids: Vec<u64> = env::var("ADMIN_USER_IDS")
            .map_err(|_| ConfigError::Missing("ADMIN_USER_IDS"))?
            .split(',')
            .map(|s| {
                s.trim()
                    .parse::<u64>()
                    .map_err(|_| ConfigError::Invalid("ADMIN_USER_IDS", "must be comma-separated u64"))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let database_url =
            env::var("DATABASE_URL").map_err(|_| ConfigError::Missing("DATABASE_URL"))?;

        let polymarket_api_base_url = env::var("POLYMARKET_API_BASE_URL")
            .unwrap_or_else(|_| "https://data-api.polymarket.com".to_string());

        let poll_interval_secs: u64 = env::var("POLL_INTERVAL_SECS")
            .unwrap_or_else(|_| "15".to_string())
            .parse()
            .map_err(|_| ConfigError::Invalid("POLL_INTERVAL_SECS", "must be a u64"))?;

        let max_concurrency: usize = env::var("MAX_CONCURRENCY")
            .unwrap_or_else(|_| "5".to_string())
            .parse()
            .map_err(|_| ConfigError::Invalid("MAX_CONCURRENCY", "must be a usize"))?;

        let http_port: u16 = env::var("HTTP_PORT")
            .unwrap_or_else(|_| "8080".to_string())
            .parse()
            .map_err(|_| ConfigError::Invalid("HTTP_PORT", "must be a u16"))?;

        Ok(Config {
            telegram_bot_token,
            admin_user_ids,
            database_url,
            polymarket_api_base_url,
            poll_interval_secs,
            max_concurrency,
            http_port,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("missing required env var: {0}")]
    Missing(&'static str),
    #[error("invalid env var {0}: {1}")]
    Invalid(&'static str, &'static str),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults_applied() {
        // Set required vars
        env::set_var("TELEGRAM_BOT_TOKEN", "test_token");
        env::set_var("ADMIN_USER_IDS", "111,222");
        env::set_var("DATABASE_URL", "postgresql://localhost/test");

        // Remove optional vars to test defaults
        env::remove_var("POLYMARKET_API_BASE_URL");
        env::remove_var("POLL_INTERVAL_SECS");
        env::remove_var("MAX_CONCURRENCY");
        env::remove_var("HTTP_PORT");

        let config = Config::from_env().unwrap();
        assert_eq!(config.polymarket_api_base_url, "https://data-api.polymarket.com");
        assert_eq!(config.poll_interval_secs, 15);
        assert_eq!(config.max_concurrency, 5);
        assert_eq!(config.http_port, 8080);
        assert_eq!(config.admin_user_ids, vec![111, 222]);
    }

    #[test]
    fn test_missing_required_var() {
        env::remove_var("TELEGRAM_BOT_TOKEN");
        let result = Config::from_env();
        assert!(result.is_err());
    }
}
