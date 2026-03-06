use crate::polymarket::Trade;
use std::collections::HashSet;

pub struct WalletMetrics {
    pub total_trades: usize,
    #[allow(dead_code)]
    pub taker_trades: usize,
    pub trades_per_day: f64,
    pub micro_trade_ratio: f64,
    pub maker_fill_ratio: f64,
    pub uniformity_24h: f64,
    pub unique_markets: usize,
    pub window_days: f64,
}

const MICRO_TRADE_USD_THRESHOLD: f64 = 5.0;

impl WalletMetrics {
    /// `all_trades`   — fetched with takerOnly=false (all fills)
    /// `taker_trades` — fetched with takerOnly=true  (only taker fills)
    pub fn compute(all_trades: &[Trade], taker_trades: &[Trade]) -> Self {
        let total = all_trades.len();
        if total == 0 {
            return Self::zero();
        }

        let min_ts = all_trades.iter().map(|t| t.timestamp).min().unwrap();
        let max_ts = all_trades.iter().map(|t| t.timestamp).max().unwrap();
        let window_days = ((max_ts - min_ts).max(1) as f64) / 86_400.0;
        let trades_per_day = total as f64 / window_days;

        let micro = all_trades
            .iter()
            .filter(|t| t.usdc_value() < MICRO_TRADE_USD_THRESHOLD)
            .count();
        let micro_trade_ratio = micro as f64 / total as f64;

        let taker_count = taker_trades.len();
        let maker_count = total.saturating_sub(taker_count);
        let maker_fill_ratio = maker_count as f64 / total as f64;

        let mut hourly = [0usize; 24];
        for t in all_trades {
            let h = ((t.timestamp % 86_400) / 3_600) as usize;
            hourly[h] += 1;
        }
        let entropy: f64 = hourly
            .iter()
            .filter(|&&c| c > 0)
            .map(|&c| {
                let p = c as f64 / total as f64;
                -p * p.ln()
            })
            .sum();
        let uniformity_24h = entropy / (24f64).ln();

        let unique_markets = all_trades
            .iter()
            .map(|t| t.condition_id.as_str())
            .collect::<HashSet<_>>()
            .len();

        Self {
            total_trades: total,
            taker_trades: taker_count,
            trades_per_day,
            micro_trade_ratio,
            maker_fill_ratio,
            uniformity_24h,
            unique_markets,
            window_days,
        }
    }

    fn zero() -> Self {
        Self {
            total_trades: 0,
            taker_trades: 0,
            trades_per_day: 0.0,
            micro_trade_ratio: 0.0,
            maker_fill_ratio: 0.0,
            uniformity_24h: 0.0,
            unique_markets: 0,
            window_days: 0.0,
        }
    }
}

pub struct RuleResult {
    #[allow(dead_code)]
    pub name: &'static str,
    pub points: u32,
    pub max_points: u32,
    pub triggered: bool,
    pub detail: String,
}

pub trait ScoringRule: Send + Sync {
    fn evaluate(&self, metrics: &WalletMetrics) -> RuleResult;
}

// ---- Concrete rules ------------------------------------------------------

pub struct TradeFrequencyRule;
pub struct MicroTradeRatioRule;
pub struct MakerFillRule;
pub struct UniformityRule;
pub struct MarketDiversityRule;
pub struct ReactionTimeRule;
pub struct CopycatClusterRule;

impl ScoringRule for TradeFrequencyRule {
    fn evaluate(&self, m: &WalletMetrics) -> RuleResult {
        let triggered = m.trades_per_day > 200.0;
        RuleResult {
            name: "Trade frequency",
            points: if triggered { 30 } else { 0 },
            max_points: 30,
            triggered,
            detail: format!("avg {:.1} trades/day (threshold: 200)", m.trades_per_day),
        }
    }
}

impl ScoringRule for MicroTradeRatioRule {
    fn evaluate(&self, m: &WalletMetrics) -> RuleResult {
        let triggered = m.micro_trade_ratio > 0.70;
        RuleResult {
            name: "Micro-trade ratio",
            points: if triggered { 25 } else { 0 },
            max_points: 25,
            triggered,
            detail: format!(
                "{:.1}% under $5 (threshold: 70%)",
                m.micro_trade_ratio * 100.0
            ),
        }
    }
}

impl ScoringRule for MakerFillRule {
    fn evaluate(&self, m: &WalletMetrics) -> RuleResult {
        let triggered = m.maker_fill_ratio > 0.70;
        RuleResult {
            name: "Maker fill ratio",
            points: if triggered { 20 } else { 0 },
            max_points: 20,
            triggered,
            detail: format!(
                "{:.1}% maker fills (threshold: 70%)",
                m.maker_fill_ratio * 100.0
            ),
        }
    }
}

impl ScoringRule for UniformityRule {
    fn evaluate(&self, m: &WalletMetrics) -> RuleResult {
        let triggered = m.uniformity_24h > 0.80;
        RuleResult {
            name: "24h uniformity",
            points: if triggered { 15 } else { 0 },
            max_points: 15,
            triggered,
            detail: format!("score: {:.2} (threshold: 0.80)", m.uniformity_24h),
        }
    }
}

impl ScoringRule for MarketDiversityRule {
    fn evaluate(&self, m: &WalletMetrics) -> RuleResult {
        let triggered = m.unique_markets > 50;
        RuleResult {
            name: "Market diversity",
            points: if triggered { 10 } else { 0 },
            max_points: 10,
            triggered,
            detail: format!("{} markets/7d (threshold: 50)", m.unique_markets),
        }
    }
}

impl ScoringRule for ReactionTimeRule {
    fn evaluate(&self, _: &WalletMetrics) -> RuleResult {
        RuleResult {
            name: "Reaction time to price jumps",
            points: 0,
            max_points: 0,
            triggered: false,
            detail: "requires price history — not implemented".to_string(),
        }
    }
}

impl ScoringRule for CopycatClusterRule {
    fn evaluate(&self, _: &WalletMetrics) -> RuleResult {
        RuleResult {
            name: "Copycat cluster",
            points: 0,
            max_points: 0,
            triggered: false,
            detail: "requires cross-wallet analysis — not implemented".to_string(),
        }
    }
}

pub struct BotScorePipeline {
    rules: Vec<Box<dyn ScoringRule>>,
}

impl Default for BotScorePipeline {
    fn default() -> Self {
        Self {
            rules: vec![
                Box::new(TradeFrequencyRule),
                Box::new(MicroTradeRatioRule),
                Box::new(MakerFillRule),
                Box::new(UniformityRule),
                Box::new(ReactionTimeRule),
                Box::new(MarketDiversityRule),
                Box::new(CopycatClusterRule),
            ],
        }
    }
}

pub enum BotLabel {
    HumanLikely,
    Hybrid,
    BotLikely,
}

pub struct BotScoreResult {
    pub score: u32,
    pub label: BotLabel,
    pub rule_results: Vec<RuleResult>,
    pub metrics: WalletMetrics,
}

impl BotScorePipeline {
    pub fn run(&self, all_trades: &[Trade], taker_trades: &[Trade]) -> BotScoreResult {
        let metrics = WalletMetrics::compute(all_trades, taker_trades);
        let rule_results: Vec<_> = self.rules.iter().map(|r| r.evaluate(&metrics)).collect();
        let score = rule_results.iter().map(|r| r.points).sum::<u32>().min(100);
        let label = match score {
            0..=25 => BotLabel::HumanLikely,
            26..=60 => BotLabel::Hybrid,
            _ => BotLabel::BotLikely,
        };
        BotScoreResult {
            score,
            label,
            rule_results,
            metrics,
        }
    }
}

pub fn format_bot_score(address: &str, result: &BotScoreResult) -> String {
    let label = match result.label {
        BotLabel::HumanLikely => "HumanLikely",
        BotLabel::Hybrid => "Hybrid",
        BotLabel::BotLikely => "BotLikely",
    };
    let short = format!("{}...{}", &address[..6], &address[address.len() - 4..]);

    let mut lines = vec![
        "<b>Bot Score Analysis</b>".to_string(),
        format!("Wallet: <code>{}</code>", short),
        String::new(),
        format!("<b>Score: {}/100 — {}</b>", result.score, label),
        String::new(),
        "<b>Signals:</b>".to_string(),
    ];

    for r in &result.rule_results {
        if r.max_points > 0 {
            let icon = if r.triggered { "[+]" } else { "[ ]" };
            let pts = if r.triggered { r.points } else { 0 };
            lines.push(format!("  {} +{}  {}", icon, pts, r.detail));
        }
    }

    lines.push(String::new());
    lines.push(format!(
        "Analysis: {} trades | {:.1} days",
        result.metrics.total_trades, result.metrics.window_days,
    ));

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_trade(timestamp: i64, size: f64, price: f64, condition_id: &str) -> Trade {
        Trade {
            proxy_wallet: "0xtest".to_string(),
            side: "BUY".to_string(),
            size,
            price,
            timestamp,
            title: "Test".to_string(),
            outcome: "Yes".to_string(),
            outcome_index: 0,
            transaction_hash: format!("0x{}", timestamp),
            condition_id: condition_id.to_string(),
            slug: "test".to_string(),
            event_slug: None,
            asset: None,
            alias: None,
        }
    }

    #[test]
    fn test_empty_trades_zero_metrics() {
        let m = WalletMetrics::compute(&[], &[]);
        assert_eq!(m.total_trades, 0);
        assert_eq!(m.trades_per_day, 0.0);
        assert_eq!(m.micro_trade_ratio, 0.0);
        assert_eq!(m.uniformity_24h, 0.0);
        assert_eq!(m.unique_markets, 0);
    }

    #[test]
    fn test_trades_per_day_calculation() {
        // 200 trades spanning exactly 1 day (86400 seconds)
        let trades: Vec<_> = (0..200)
            .map(|i| make_trade(1000 + i * 432, 10.0, 0.5, "cond_a"))
            .collect();
        let m = WalletMetrics::compute(&trades, &trades);
        assert!(
            (m.trades_per_day - 200.0).abs() < 5.0,
            "expected ~200, got {}",
            m.trades_per_day
        );
    }

    #[test]
    fn test_micro_trade_ratio() {
        let mut trades = vec![];
        // 8 trades under $5 (size=1, price=0.5 => $0.50)
        for i in 0..8 {
            trades.push(make_trade(1000 + i * 100, 1.0, 0.50, "cond_a"));
        }
        // 2 trades over $5 (size=100, price=0.5 => $50)
        for i in 0..2 {
            trades.push(make_trade(2000 + i * 100, 100.0, 0.50, "cond_a"));
        }
        let m = WalletMetrics::compute(&trades, &trades);
        assert!(
            (m.micro_trade_ratio - 0.8).abs() < 0.01,
            "expected 0.8, got {}",
            m.micro_trade_ratio
        );
    }

    #[test]
    fn test_maker_fill_ratio() {
        let all: Vec<_> = (0..10)
            .map(|i| make_trade(1000 + i * 100, 10.0, 0.5, "cond_a"))
            .collect();
        // Only 2 taker trades => 8 maker => ratio = 0.8
        let taker = all[..2].to_vec();
        let m = WalletMetrics::compute(&all, &taker);
        assert!(
            (m.maker_fill_ratio - 0.8).abs() < 0.01,
            "expected 0.8, got {}",
            m.maker_fill_ratio
        );
    }

    #[test]
    fn test_uniformity_single_hour() {
        // All trades in the same hour => low uniformity
        let trades: Vec<_> = (0..100)
            .map(|i| make_trade(3600 * 14 + i, 10.0, 0.5, "cond_a"))
            .collect();
        let m = WalletMetrics::compute(&trades, &trades);
        assert!(
            m.uniformity_24h < 0.1,
            "expected near 0, got {}",
            m.uniformity_24h
        );
    }

    #[test]
    fn test_uniformity_spread_across_24h() {
        // 1 trade per hour for 24 hours => high uniformity
        let trades: Vec<_> = (0..24)
            .map(|h| make_trade(h * 3600, 10.0, 0.5, "cond_a"))
            .collect();
        let m = WalletMetrics::compute(&trades, &trades);
        assert!(
            (m.uniformity_24h - 1.0).abs() < 0.01,
            "expected ~1.0, got {}",
            m.uniformity_24h
        );
    }

    #[test]
    fn test_unique_markets() {
        let trades = vec![
            make_trade(1000, 10.0, 0.5, "cond_a"),
            make_trade(1100, 10.0, 0.5, "cond_b"),
            make_trade(1200, 10.0, 0.5, "cond_a"),
            make_trade(1300, 10.0, 0.5, "cond_c"),
        ];
        let m = WalletMetrics::compute(&trades, &trades);
        assert_eq!(m.unique_markets, 3);
    }

    #[test]
    fn test_label_human_likely() {
        let pipeline = BotScorePipeline::default();

        // Few trades spread over many days, large values, single market
        // => no rules fire => score 0 => HumanLikely
        let trades: Vec<_> = (0..5)
            .map(|i| make_trade(i * 86_400, 100.0, 0.50, "cond_a"))
            .collect();
        let result = pipeline.run(&trades, &trades);
        assert_eq!(result.score, 0);
        assert!(matches!(result.label, BotLabel::HumanLikely));
    }

    #[test]
    fn test_trade_frequency_rule_triggers() {
        let pipeline = BotScorePipeline::default();
        // 300 trades in ~1 day => triggers trade frequency (+30)
        let trades: Vec<_> = (0..300)
            .map(|i| make_trade(1000 + i * 288, 10.0, 0.5, "cond_a"))
            .collect();
        let result = pipeline.run(&trades, &trades);
        assert!(result.score >= 30);
    }

    #[test]
    fn test_all_rules_fire_clamped_to_100() {
        let pipeline = BotScorePipeline::default();

        // Build trades that trigger rules 1, 2, 4, 6
        // 300 trades spanning 1 day, all micro (<$5), spread across 24h, 60 markets
        let mut trades = vec![];
        for i in 0..300 {
            let hour = i % 24;
            let market = format!("cond_{}", i % 60);
            trades.push(make_trade(
                hour as i64 * 3600 + (i / 24) as i64,
                1.0,
                0.50,
                &market,
            ));
        }
        // Taker trades = 0 => maker_fill_ratio = 1.0 (triggers rule 3)
        let taker: Vec<Trade> = vec![];
        let result = pipeline.run(&trades, &taker);
        assert!(
            result.score <= 100,
            "score should be clamped to 100, got {}",
            result.score
        );
        assert!(matches!(result.label, BotLabel::BotLikely));
    }

    #[test]
    fn test_format_bot_score_output() {
        let pipeline = BotScorePipeline::default();
        let trades = vec![make_trade(1000, 10.0, 0.5, "cond_a")];
        let result = pipeline.run(&trades, &trades);
        let output = format_bot_score("0x1234567890abcdef1234567890abcdef12345678", &result);
        assert!(output.contains("Bot Score Analysis"));
        assert!(output.contains("0x1234...5678"));
        assert!(output.contains("Score:"));
        assert!(output.contains("Signals:"));
    }
}
