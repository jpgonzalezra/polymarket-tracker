use crate::polymarket::Trade;
use std::collections::HashSet;

/// Filter trades to only include new ones that occurred after `startup_timestamp`
/// and haven't been seen yet (by tx hash).
pub fn filter_new_trades(
    trades: &[Trade],
    startup_timestamp: i64,
    seen_tx_hashes: &HashSet<String>,
) -> Vec<Trade> {
    let mut new_trades: Vec<Trade> = trades
        .iter()
        .filter(|t| t.timestamp > startup_timestamp && !seen_tx_hashes.contains(&t.transaction_hash))
        .cloned()
        .collect();

    // Sort ascending by timestamp so we process oldest first
    new_trades.sort_by_key(|t| t.timestamp);
    new_trades
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_trade(timestamp: i64, tx_hash: &str) -> Trade {
        Trade {
            proxy_wallet: "0xtest".to_string(),
            side: "BUY".to_string(),
            size: 10.0,
            price: 0.5,
            timestamp,
            title: "Test market".to_string(),
            outcome: "Yes".to_string(),
            outcome_index: 0,
            transaction_hash: tx_hash.to_string(),
            condition_id: "0xcond".to_string(),
            slug: "test".to_string(),
            event_slug: None,
            asset: None,
        }
    }

    #[test]
    fn test_all_old_trades_filtered() {
        let trades = vec![make_trade(100, "0xa"), make_trade(200, "0xb")];
        let seen = HashSet::new();
        let result = filter_new_trades(&trades, 300, &seen);
        assert!(result.is_empty());
    }

    #[test]
    fn test_mix_of_old_and_new() {
        let trades = vec![
            make_trade(100, "0xa"),
            make_trade(400, "0xb"),
            make_trade(500, "0xc"),
        ];
        let seen = HashSet::new();
        let result = filter_new_trades(&trades, 300, &seen);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].transaction_hash, "0xb");
        assert_eq!(result[1].transaction_hash, "0xc");
    }

    #[test]
    fn test_seen_tx_hashes_filtered() {
        let trades = vec![
            make_trade(400, "0xa"),
            make_trade(500, "0xb"),
            make_trade(600, "0xc"),
        ];
        let seen: HashSet<String> = ["0xa".to_string(), "0xc".to_string()].into();
        let result = filter_new_trades(&trades, 300, &seen);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].transaction_hash, "0xb");
    }

    #[test]
    fn test_empty_trades() {
        let trades: Vec<Trade> = vec![];
        let seen = HashSet::new();
        let result = filter_new_trades(&trades, 300, &seen);
        assert!(result.is_empty());
    }

    #[test]
    fn test_all_new_trades_sorted_ascending() {
        let trades = vec![
            make_trade(600, "0xc"),
            make_trade(400, "0xa"),
            make_trade(500, "0xb"),
        ];
        let seen = HashSet::new();
        let result = filter_new_trades(&trades, 300, &seen);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].timestamp, 400);
        assert_eq!(result[1].timestamp, 500);
        assert_eq!(result[2].timestamp, 600);
    }

    #[test]
    fn test_exact_startup_timestamp_excluded() {
        // Trade at exactly startup_timestamp should NOT be included
        let trades = vec![make_trade(300, "0xa"), make_trade(301, "0xb")];
        let seen = HashSet::new();
        let result = filter_new_trades(&trades, 300, &seen);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].transaction_hash, "0xb");
    }
}
