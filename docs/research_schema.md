# Research Schema

This document defines the exact v1 schemas for Rust research exports, Qlib input,
and score files consumed back by Rust.

## 1. Dataset versions

- Feature export version: `features_v1`
- Manifest version: `manifest_v1`
- Score export version: `scores_v1`

All files must embed a schema version string.

## 2. Feature parquet schema

File name convention:

```text
data/research/features/<asset>_<duration>_<start>_<end>_features_v1.parquet
```

Primary key:

- `condition_id: string`
- `ts: int64`

Partitioning convention:

- first partition by `asset`
- second partition by `duration`
- then by date if exports are directory-based

### 2.1 Identity columns

| Column | Type | Notes |
|---|---|---|
| `schema_version` | string | Always `features_v1` |
| `condition_id` | string | Canonical market key |
| `market_slug` | string | Original Polymarket slug |
| `instrument` | string | Same as `condition_id` in v1 |
| `asset` | string | `btc`, `eth`, `sol`, `xrp` |
| `duration` | string | `5m`, `15m`, `1h` |
| `market_family` | string | `updown_open_close` or `threshold_at_expiry` |
| `market_start_ts` | int64 | UTC seconds |
| `market_end_ts` | int64 | UTC seconds |
| `ts` | int64 | Observation time in UTC seconds |

### 2.2 Book state columns

| Column | Type | Notes |
|---|---|---|
| `yes_bid` | f64 | Best bid |
| `yes_ask` | f64 | Best ask |
| `no_bid` | f64 | Best bid |
| `no_ask` | f64 | Best ask |
| `yes_mid` | f64 | `(yes_bid + yes_ask) / 2` |
| `no_mid` | f64 | `(no_bid + no_ask) / 2` |
| `yes_spread` | f64 | `yes_ask - yes_bid` |
| `no_spread` | f64 | `no_ask - no_bid` |
| `book_sum` | f64 | `yes_ask + no_ask` |
| `book_gap` | f64 | `book_sum - 1.0` |

### 2.3 Optional depth columns

If PMXT export includes depth fields, populate them. Otherwise write nulls.

| Column | Type |
|---|---|
| `yes_bid_depth_1` | f64 |
| `yes_ask_depth_1` | f64 |
| `no_bid_depth_1` | f64 |
| `no_ask_depth_1` | f64 |
| `yes_bid_depth_5` | f64 |
| `yes_ask_depth_5` | f64 |
| `no_bid_depth_5` | f64 |
| `no_ask_depth_5` | f64 |

### 2.4 Market-state columns

| Column | Type | Notes |
|---|---|---|
| `age_s` | i32 | Seconds since market open |
| `time_remaining_s` | i32 | Seconds until market end |
| `in_entry_window` | bool | True if new entries are allowed |
| `in_exit_only_window` | bool | True if only exits are allowed |
| `tick_size` | f64 | Price tick |
| `fees_enabled` | bool | From metadata when available |

### 2.5 Derived microstructure features

| Column | Type |
|---|---|
| `yes_mid_ret_1s` | f64 |
| `yes_mid_ret_5s` | f64 |
| `yes_mid_ret_15s` | f64 |
| `yes_mid_ret_30s` | f64 |
| `no_mid_ret_1s` | f64 |
| `no_mid_ret_5s` | f64 |
| `no_mid_ret_15s` | f64 |
| `no_mid_ret_30s` | f64 |
| `book_gap_z_30s` | f64 |
| `yes_spread_ema_15s` | f64 |
| `no_spread_ema_15s` | f64 |
| `quote_updates_5s` | i32 |
| `price_changes_5s` | i32 |
| `realized_vol_15s` | f64 |
| `realized_vol_30s` | f64 |
| `realized_vol_60s` | f64 |

### 2.6 Optional spot/reference columns

These are null in PMXT-only exports and populated only when `--with-spot` is used.

| Column | Type |
|---|---|
| `spot_price` | f64 |
| `spot_ret_1s` | f64 |
| `spot_ret_5s` | f64 |
| `spot_ret_15s` | f64 |
| `spot_realized_vol_30s` | f64 |
| `spot_distance_to_strike` | f64 |
| `spot_z_to_strike` | f64 |

## 3. Label columns

Labels live in the same parquet file in v1 for simplicity.

| Column | Type | Definition |
|---|---|---|
| `label_reachable_yes_15s` | i8 | `1` if max executable YES exit bid in next 15s is profitable after fees |
| `label_reachable_yes_30s` | i8 | Same for 30s |
| `label_reachable_yes_45s` | i8 | Same for 45s |
| `label_reachable_no_15s` | i8 | Same for NO |
| `label_reachable_no_30s` | i8 | Same for NO |
| `label_reachable_no_45s` | i8 | Same for NO |
| `label_edge_yes_15s` | f64 | Max net YES edge in next 15s |
| `label_edge_yes_30s` | f64 | Max net YES edge in next 30s |
| `label_edge_yes_45s` | f64 | Max net YES edge in next 45s |
| `label_edge_no_15s` | f64 | Max net NO edge in next 15s |
| `label_edge_no_30s` | f64 | Max net NO edge in next 30s |
| `label_edge_no_45s` | f64 | Max net NO edge in next 45s |
| `label_adverse_yes_30s` | f64 | Worst YES drawdown in next 30s after costs |
| `label_adverse_no_30s` | f64 | Worst NO drawdown in next 30s after costs |
| `label_resolution` | i8 | Optional final outcome label for analysis only |

### 3.1 Label math

For YES at time `t`:

- entry price = `yes_ask_t`
- exit candidates = future `yes_bid` values in `(t, t+h]`
- net edge = `future_yes_bid - yes_ask_t - fee_open - fee_close - slippage_buffer`

For NO at time `t`:

- entry price = `no_ask_t`
- exit candidates = future `no_bid` values in `(t, t+h]`
- net edge = `future_no_bid - no_ask_t - fee_open - fee_close - slippage_buffer`

## 4. Manifest JSON schema

File name convention:

```text
data/research/manifests/<asset>_<duration>_<start>_<end>_manifest_v1.json
```

Required fields:

| Field | Type |
|---|---|
| `schema_version` | string |
| `features_path` | string |
| `asset` | string |
| `duration` | string |
| `market_family` | string |
| `start_ts` | int64 |
| `end_ts` | int64 |
| `resolution` | string |
| `source_inputs` | array[string] |
| `train_start_ts` | int64 |
| `train_end_ts` | int64 |
| `valid_start_ts` | int64 |
| `valid_end_ts` | int64 |
| `test_start_ts` | int64 |
| `test_end_ts` | int64 |
| `label_columns` | array[string] |
| `feature_columns` | array[string] |

## 5. Score parquet schema

File name convention:

```text
data/research/scores/<model_name>_<asset>_<duration>_scores_v1.parquet
```

Required columns:

| Column | Type | Notes |
|---|---|---|
| `schema_version` | string | Always `scores_v1` |
| `condition_id` | string | Match feature parquet |
| `ts` | int64 | UTC seconds |
| `model_version` | string | Champion/challenger version |
| `score_yes_15s` | f64 | Probability or normalized score |
| `score_yes_30s` | f64 | Probability or normalized score |
| `score_yes_45s` | f64 | Probability or normalized score |
| `score_no_15s` | f64 | Probability or normalized score |
| `score_no_30s` | f64 | Probability or normalized score |
| `score_no_45s` | f64 | Probability or normalized score |
| `risk_yes_30s` | f64 | Adverse-risk score |
| `risk_no_30s` | f64 | Adverse-risk score |
| `fresh_until_ts` | int64 | TTL cutoff used by Rust |

## 6. Rust fusion expectations

Rust consumes score parquet only if:

- `schema_version == scores_v1`
- `condition_id` matches the active market
- `ts` is aligned to the observation timestamp
- `fresh_until_ts >= current_ts`
- required score columns are present

If any of the above fail:

- fused mode blocks or degrades according to config
- heuristic mode remains available

## 7. Qlib input contract

Qlib v1 uses the exported parquet directly.

- instrument column: `instrument`
- datetime column: `ts`
- labels: all `label_*`
- features: all non-label numeric columns except identifiers

The first Qlib handler must not assume OHLCV stock-style fields beyond what this
schema explicitly exports.
