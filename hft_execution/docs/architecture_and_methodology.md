# Architecture and Methodology

## System Architecture

The system is structured as a pipeline:

1. **Market Data Handler** (`market_data.rs`)
   - Maintains a local order book and recent trades per instrument.
   - Provides mid-price, spread, imbalance, and queue estimates.

2. **Signal Generator** (`signals.rs`)
   - Aggregates microstructure signals into a single `Signals` struct.

3. **Decision Logic** (`decision.rs`)
   - Translates signals + urgency into passive or aggressive order actions.
   - Applies child sizing and post-only logic.

4. **Execution Engine** (`execution.rs`)
   - Places/cancels orders, enforces TTL, and tracks remaining quantity.
   - Updates internal order state based on fills.

5. **Performance Tracker** (`performance.rs`)
   - Calculates implementation shortfall, VWAP comparison, fill rate,
     adverse selection, and maker/taker ratio.
   - Writes per-fill CSV logs and a summary report.

6. **Exchange Interface** (`exchange.rs`)
   - Simulation environment that emits book/trade events and fills.
   - Designed to be swapped with a live testnet implementation.

## Microstructure Signals

The algorithm uses non-ML signals derived from order book and trade flow:

- **Order book imbalance:** top 5 levels bid/ask volume ratio.
- **Queue position estimation:** depth at our price to estimate wait time.
- **Large resting orders:** levels > 2.5x average depth.
- **Iceberg detection:** repeated small trades at the same price.
- **Spread dynamics:** spread in bps to decide post-only vs crossing.
- **Trade flow imbalance:** buy vs sell volume in recent trades.
- **Short-term momentum:** drift in recent trade prices.
- **Mean reversion:** deviation after large trades.

## Adverse Selection Detection

Adverse selection is handled by:

- **Toxic flow detection:** high flow imbalance + widening spread.
- **Stale order cancellation:** TTL-based cancels to avoid late fills.
- **Signal-aware aggression:** reduce passive exposure when flow is toxic.

Adverse selection is also **measured** by the average mid-price movement
after fills at 1s / 5s / 30s.

## Execution Decision Logic

The decision engine balances price improvement and fill urgency:

- **Passive placement** when spreads are wide and flow is benign.
- **Aggressive crossing** when urgency is high or momentum goes against us.
- **Post-only** enabled only if spread is above a configured threshold.
- **Child sizing** capped by top-of-book liquidity.

## Performance Tracking

Metrics tracked per instrument:

- Implementation shortfall (bps) vs decision price.
- VWAP comparison (bps) vs market trades during the window.
- Fill rate vs target quantity.
- Maker vs taker fill ratios.
- Adverse selection (mid-price move after fills).

## Assumptions

- The simulator provides realistic microstructure conditions.
- Queue position is approximated by displayed depth at our price.
- Trades are aggregated in small windows (seconds).
- For live use, WebSocket feeds must provide best bid/ask and trades.

## Cursor / AI Development Strategy

Example approach when using Cursor:

- Prompt for module scaffolding and function signatures.
- Ask for microstructure signal definitions and formulas.
- Use AI to draft tests for decision logic edge cases.
- Iterate on execution behavior by reviewing logs and IS metrics.
