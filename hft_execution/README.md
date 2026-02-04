# HFT Execution Algorithm (Simulation Scaffold)

This project implements a microstructure-driven execution algorithm that targets
negative implementation shortfall (IS) while maintaining high fill rates. It
includes a full simulation loop (market data + matching) so that signals and
performance reporting can be validated without external dependencies.

The code is designed to be swapped to a real testnet exchange by replacing the
`MockExchange` with a live WebSocket + REST implementation.

## Architecture

```
MarketData (book + trades)
        │
        ▼
Signals (imbalance, flow, spread, icebergs, momentum)
        │
        ▼
Decision Engine (passive vs aggressive, sizing, TTL)
        │
        ▼
Execution Engine (place/cancel/track orders)
        │
        ▼
Performance Tracker (IS, VWAP, fill rate, adverse selection)
```

### Core Modules

- `market_data.rs`: Maintains book + trades, computes microstructure inputs.
- `signals.rs`: Aggregates imbalance, flow, spread, momentum, iceberg detection.
- `decision.rs`: Chooses passive vs aggressive orders and child sizing.
- `execution.rs`: Places/cancels orders, enforces TTL, updates fills.
- `exchange.rs`: Simulation exchange (market data + fills). Replace for live.
- `performance.rs`: IS, VWAP, fill rate, adverse selection, maker/taker metrics.
- `report.rs`: Outputs a Markdown report.

## Microstructure Signals

Implemented signals include:

- **Order book imbalance** (top 5 levels).
- **Queue position estimation** (order book size at price).
- **Large resting orders** (size vs. average depth).
- **Iceberg detection** (repeated small trades at same price).
- **Spread dynamics** (basis points).
- **Trade flow imbalance** (buy vs sell volume).
- **Short-term momentum** (trade price drift).
- **Mean reversion** after large trades.

## Execution Tactics

- Passive vs aggressive mix based on urgency and signals.
- Post-only orders when spreads are wide and flow is benign.
- Smart crossing when urgency or momentum indicates risk of missing fills.
- Child order sizing based on top-of-book liquidity.
- Stale order cancellation via TTL to reduce adverse selection.

## Performance Tracking

The tracker logs every fill to CSV and reports:

- Implementation shortfall (IS) in bps.
- VWAP comparison vs market trades in the execution window.
- Fill rate by instrument.
- Adverse selection (price move after 1s/5s/30s).
- Maker vs taker fill ratio.

Outputs:

- `reports/fills.csv`
- `reports/performance_report.md`

## Running the Simulation

```
cd hft_execution
cargo run -- config/example_config.yaml
```

## Configuration

Edit `config/example_config.yaml` to change:

- Instruments (spot, perp, option).
- Target sizes and side.
- Execution window and decision cadence.
- Simulation parameters (spread, volatility, liquidity).

## Live Exchange Integration (Stubbed)

To use a real testnet:

1. Implement an exchange client in `exchange.rs` using WebSocket market data and
   REST order placement.
2. Replace `MockExchange` with your live client in `main.rs`.
3. Map book/trade events into `MarketDataEvent`.
4. Emit fills into the `ExecutionEngine`.

## Cursor / AI Assistance Notes

Example workflow:

1. Ask Cursor to outline architecture and data flows.
2. Generate module skeletons (config, market data, execution, performance).
3. Iterate on microstructure heuristics and unit tests.
4. Use inline explanations to verify adverse-selection logic.

This keeps the development flow fast and reproducible.
