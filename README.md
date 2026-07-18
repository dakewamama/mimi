# Mimi

Autonomous agent that catches Solana prediction markets mispricing against
the TxODDS sharp line, and broadcasts the divergence as a live signal.

## What it does
1. Listens to the TxLINE feed (professional sharp line).
2. Prices fair value by stripping the bookmaker margin (de-vig).
3. Reads the live on-chain price on Jupiter Predict (open markets only).
4. Detects divergence and emits a Buy/Sell signal past a threshold.
5. Broadcasts every signal over an HTTP feed: GET /signals, GET /health.

Users consume the signal feed with no wallet and no execution rights.
Execution and open infrastructure are the roadmap.

## Run
    cp .env.example .env      # fill TXLINE_API_TOKEN, SOLANA_KEYPAIR_PATH
    cargo test
    cargo run                 # serves the signal feed on MIMI_BIND (default 0.0.0.0:8080)

## Layout
    src/pricing.rs     odds -> fair value (de-vig), invariant-tested
    src/feed.rs        TxLINE source (live guest auth confirmed)
    src/venue.rs       Jupiter Predict client, open-market filtered
    src/divergence.rs  fair-vs-venue edge detection
    src/wallet.rs      local signer, keypair path only
    src/api.rs         HTTP signal feed
    src/main.rs        concurrent detection loop + feed
    idl/txoracle.json  TxODDS on-chain program IDL (fetched from devnet)

