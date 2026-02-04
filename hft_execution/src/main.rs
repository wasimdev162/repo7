mod config;
mod decision;
mod exchange;
mod execution;
mod market_data;
mod performance;
mod report;
mod signals;
mod types;

use std::fs;
use std::path::Path;

use tracing::{info, warn};

use crate::config::{Config, RuntimeMode};
use crate::decision::DecisionEngine;
use crate::exchange::MockExchange;
use crate::execution::ExecutionEngine;
use crate::performance::PerformanceTracker;
use crate::report::write_markdown_report;

#[tokio::main]
async fn main() {
    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "config/example_config.yaml".to_string());
    let config = match load_config(&config_path) {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("failed to load config: {}", err);
            std::process::exit(1);
        }
    };

    init_logging(&config.logging.log_level);
    ensure_parent_dir(&config.logging.csv_fill_log_path);
    ensure_parent_dir(&config.logging.report_path);

    let performance = match PerformanceTracker::new(&config.logging.csv_fill_log_path) {
        Ok(tracker) => tracker,
        Err(err) => {
            eprintln!("failed to create fill log: {}", err);
            std::process::exit(1);
        }
    };

    if let RuntimeMode::Simulation = config.runtime.mode {
        let exchange = MockExchange::new(
            config.simulation.clone(),
            config
                .instruments
                .iter()
                .map(|cfg| cfg.instrument.clone())
                .collect(),
        )
        .start();

        let decision_engine = DecisionEngine::new(config.execution.clone());
        let engine = ExecutionEngine::new(
            exchange,
            decision_engine,
            performance,
            config.execution.clone(),
            config.instruments.clone(),
        );

        let reports = engine.run().await;
        if let Err(err) = write_markdown_report(&config.logging.report_path, &reports) {
            warn!("failed to write report: {}", err);
        } else {
            info!("report written to {}", config.logging.report_path);
        }
    } else {
        warn!("live mode not implemented; configure RuntimeMode::Simulation");
    }
}

fn load_config(path: &str) -> Result<Config, Box<dyn std::error::Error>> {
    let contents = fs::read_to_string(path)?;
    let config: Config = serde_yaml::from_str(&contents)?;
    Ok(config)
}

fn init_logging(level: &str) {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(level));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}

fn ensure_parent_dir(path: &str) {
    if let Some(parent) = Path::new(path).parent() {
        if !parent.as_os_str().is_empty() {
            let _ = fs::create_dir_all(parent);
        }
    }
}
