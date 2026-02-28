# Hammer

> The linter for EIP-2930 access lists.

[Installation](#installation) · [Usage](#usage) · [Why](#why) · [Architecture](#architecture) · [References](#references)

---

Generate, validate, and compare EIP-2930 access lists against actual EVM execution traces. Built on [revm](https://github.com/bluealloy/revm) and [alloy](https://github.com/alloy-rs/alloy).

- **`hammer generate`** — Trace a transaction via revm, produce the optimal access list with warm-address stripping that existing clients miss.
- **`hammer validate`** — Diff a declared access list against the traced optimal, report missing entries, stale entries, redundant entries, and gas waste.
- **`hammer compare`** — Fetch a mined transaction by hash, extract its access list, validate it against a fresh trace, and score its optimality.

## The Problem

Only **1.46%** of Ethereum transactions include an access list, even though **42.6%** would benefit from one. Of those that do, **19.6% are suboptimal** — and 11.8% actually **cost more gas** than using no list at all.

`eth_createAccessList` ships with every node, but every major client gets the warm-address stripping wrong in different ways:

| Client     | Fails to remove  |
| ---------- | ---------------- |
| Geth       | `tx.to`          |
| Nethermind | `block.coinbase` |
| Besu       | Nothing removed  |

Hammer traces execution through revm, then strips all warm-by-default addresses: `tx.from`, `tx.to`, `block.coinbase`, precompiles (`0x01`–`0x0a`), and contracts created during the transaction. Every entry left in the list saves gas. Every entry removed prevents waste.

### Real-World Example

Running `hammer compare` on a [jaredfromsubway MEV bot](https://etherscan.io/address/0xae2fc483527b8ef99eb5d9b44875f005ba1fae13) transaction:

```
Access list optimality: 38.8%
Issues: 7 entries
  Stale: WBTC token — 1 slot — 1,900 gas wasted
  Stale: WBTC/USDT pool — 5 slots — 11,900 gas wasted
  Stale: WETH token — 1 slot — 1,900 gas wasted
  Stale: USDT/ETH pool — 5 slots — 11,900 gas wasted
  Stale: USDT token — 7 slots — 15,700 gas wasted
  Incomplete: WBTC/ETH pool — 15 missing slots — 30,000 gas wasted
  Stale: WBTC/ETH pool — 10 slots — 19,000 gas wasted

Total waste: 92,300 gas (28.9% of gas used)
```

One of Ethereum's highest-frequency MEV bots, running with a 61% suboptimal access list.

## Installation

```sh
cargo install --path cli
```

Or build from source:

```sh
git clone https://github.com/RankJay/hammer.git
cd hammer
cargo build --release
```

Requires an Ethereum RPC endpoint (Alchemy, Infura, QuickNode, etc.) for state access.

## Usage

### Generate an optimal access list

```sh
hammer generate \
  --rpc-url https://eth-mainnet.g.alchemy.com/v2/YOUR_KEY \
  --from 0xSENDER \
  --to 0xCONTRACT \
  --data 0xCALLDATA \
  --block latest \
  --output json
```

### Validate a declared access list

```sh
hammer validate \
  --rpc-url https://eth-mainnet.g.alchemy.com/v2/YOUR_KEY \
  --from 0xSENDER \
  --to 0xCONTRACT \
  --data 0xCALLDATA \
  --access-list ./my-access-list.json \
  --output human
```

Exit code `0` if valid, `1` if issues found. Designed for CI pipelines.

### Compare a mined transaction

```sh
hammer compare \
  --rpc-url https://eth-mainnet.g.alchemy.com/v2/YOUR_KEY \
  --tx-hash 0x2af76856a4ac004647e487097b82adc660747544ed7c51ede51024f16685d160
```

Fetches the transaction, extracts its declared access list, re-traces execution, and reports optimality.

## Why

### The gas math

```
Including a slot in the access list       costs  1,900 gas upfront
If that slot IS accessed during execution  saves  2,000 gas (cold→warm)
                                           ─────────────────────────────
Net gain per correctly included slot:        100 gas

If the slot is NOT accessed:              wastes  1,900 gas for nothing
```

An address in the list costs 2,400 gas. A cold address access costs 2,600. Net gain if accessed: 200 gas. Including an already-warm address (tx.from, tx.to, coinbase, precompiles) costs 2,400 gas for zero benefit.

### Who this is for

| Segment              | Why they care                                                                  |
| -------------------- | ------------------------------------------------------------------------------ |
| **MEV searchers**    | Compete on gas efficiency. 200 gas can flip a bundle's profitability.          |
| **DEX aggregators**  | Ship hardcoded access lists in SDKs. Stale lists = users overpay.              |
| **Block builders**   | Optimal access lists directly impact block profitability.                      |
| **Wallet providers** | Auto-optimize before sending. Transparent savings for users.                   |
| **Protocol teams**   | Proxy upgrades and storage migrations silently invalidate cached access lists. |

## Architecture

```
cli  →  core  →  revm + alloy
```

**`core`** is a library crate. No async, no CLI dependencies. Takes a `revm::Database`, transaction env, and block env — returns typed results. Embeddable in Foundry plugins, WASM modules, SDK middleware, or monitoring services.

**`cli`** is a thin clap wrapper. Handles RPC provider setup, async runtime, and output formatting. The CLI is a consumer of the library, not the product.

### Module map

| Module         | Purpose                                                                                   |
| -------------- | ----------------------------------------------------------------------------------------- |
| `tracer.rs`    | `HammerInspector` — revm Inspector impl. Hooks SLOAD/SSTORE/CALL/CREATE opcodes.             |
| `optimizer.rs` | Warm-address stripping. Removes tx.from, tx.to, coinbase, precompiles, created contracts. |
| `validator.rs` | Set diff between declared and actual. Categorizes: missing, stale, incomplete, redundant. |
| `gas.rs`       | EIP-2929/2930 constants and gas math. Pure functions.                                     |
| `types.rs`     | `ValidationReport`, `DiffEntry`, `GasSummary`, `OptimizedAccessList`.                     |

### Design decisions

- **Deterministic output.** `BTreeSet`/`BTreeMap` everywhere. Same input → same output. No `HashMap`.
- **revm as the EVM, not a remote node.** Full control over Inspector hooks. No trust in the node's `eth_createAccessList` implementation.
- **Library-first.** Every feature is a typed function in hammer-core. The CLI is a shell.
- **No async in the core.** revm execution is synchronous. Async lives at the CLI/integration boundary only.

## References

- [EIP-2930: Optional access lists](https://eips.ethereum.org/EIPS/eip-2930)
- [EIP-2929: Gas cost increases for state access opcodes](https://eips.ethereum.org/EIPS/eip-2929)
- [Dissecting EIP-2930 Access Lists (arXiv:2312.06574)](https://arxiv.org/abs/2312.06574) — the 2023 paper quantifying mainnet access list waste

## License

[PolyForm Noncommercial License 1.0.0](LICENSE) — free for noncommercial use.
