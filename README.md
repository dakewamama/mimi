# Mimi

Mimi is an autonomous agent that catches Solana prediction markets the moment
they misprice against the professional sharp line, and broadcasts that gap as a
live signal.

When something happens in a match, the sharp odds move within moments. On-chain
prediction markets lag. For a short window the on-chain price is wrong, and that
window is the edge. Mimi watches both sides at once and fires the instant they
diverge. A poacher scores by being first to the loose ball; Mimi is first to the
loose price.

## How it works

1. **Listen.** Subscribe to the TxLINE feed, the professional sharp line from
   TxODDS.
2. **Price.** Strip the bookmaker margin to recover the true fair probability
   (proportional de-vig).
3. **Read the chain.** Pull the live price from Jupiter Predict, filtered to
   open markets only.
4. **Detect.** Compare fair price against the on-chain price and emit a Buy or
   Sell signal once the gap clears a threshold.
5. **Broadcast.** Serve every signal over an HTTP feed. Consumers read the alert
   with no wallet and no execution rights.

## What you get today

A running detection-and-broadcast agent. It authenticates against TxLINE live,
reads live on-chain prices, detects divergence, and serves the result:

- `GET /signals` — recent divergences as JSON, newest first
- `GET /health` — liveness probe

Execution on the user's behalf, and opening the feed as infrastructure other
agents build on, are the roadmap. The signal is the product now.

## Architecture

Each stage sits behind a trait, so a data source is a swappable input and the
detection core never changes.

```
TxLINE feed ──> pricing ──> divergence ──> signal feed (HTTP)
                              ^
        Jupiter Predict ──────┘
```

```
src/pricing.rs     decimal odds to fair value, de-vig, invariant-tested
src/feed.rs        TxLINE source, live guest auth confirmed
src/venue.rs       Jupiter Predict client, open-market filtered
src/divergence.rs  fair-versus-venue edge detection
src/wallet.rs      local signer, keypair path only, never raw key material
src/api.rs         HTTP signal feed
src/main.rs        concurrent detection loop and feed
idl/txoracle.json  TxODDS on-chain program IDL, fetched from devnet
```

## Run

```
cp .env.example .env      # fill in the values below
cargo test                # 18 tests, including live TxLINE and Jupiter reads
cargo run                 # serves the feed on MIMI_BIND (default 0.0.0.0:8080)
```

Then read the feed:

```
curl localhost:8080/health     # ok
curl localhost:8080/signals    # [] until the odds feed is activated
```

## Configuration

Set in `.env` (see `.env.example`):

| Variable | Purpose |
| --- | --- |
| `TXLINE_API_TOKEN` | TxLINE data token from local activation |
| `TXLINE_ODDS_PATH` | TxLINE odds endpoint path |
| `SOLANA_KEYPAIR_PATH` | Path to a local keypair file for the signer |
| `MIMI_BIND` | Feed bind address, optional, defaults to `0.0.0.0:8080` |

Keys are referenced only as file paths, never inline. The signer reads a
standard 64-byte Solana keypair file directly and only ever logs the public key.
`.env` and any keypair file are gitignored.

## Design notes

- **Traits as seams.** `MarketSource` and `Venue` are traits; the live feed and
  on-chain venue are implementations. Detection, pricing, and the feed never
  need to know which source is behind them.
- **Open markets only.** A resolved market reports a zeroed book. Reading that
  against a real fair price would fabricate a divergence, so the venue returns
  nothing for any market not in open status.
- **Proportional de-vig.** Chosen for the first pass. Shin is the upgrade if
  favorite-longshot skew starts turning real edges into pricing artifacts.
- **No custody.** The feed exposes the alert only. No user wallet is connected
  and no execution rights are granted.

## Stack

Rust (edition 2024), tokio, axum, reqwest on rustls (no OpenSSL in the tree),
ed25519-dalek. Built for the Superteam Earn World Cup hackathon, Trading Tools
and Agents track.