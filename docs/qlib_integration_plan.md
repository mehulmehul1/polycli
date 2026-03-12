# Qlib + PMXT Integration Plan

## 1. Goal

Add `microsoft/qlib` to this repository as a research and model-lifecycle layer for
Polymarket crypto binary markets while keeping Rust as the source of truth for:

- market discovery
- live feed normalization
- risk gating
- execution and settlement
- replay truth and fee-aware PnL accounting

Qlib is not part of the live critical path in v1. It produces datasets, trained
artifacts, evaluation reports, and optional scores that Rust may consume.

## 2. Final Architecture

### 2.1 Rust responsibilities

- Normalize PMXT archive data and live market data into one canonical shape.
- Export research-ready feature and label tables.
- Simulate fees, slippage, and fills in replay.
- Load Qlib score files and fuse them with the Rust strategy engine.
- Fall back to heuristic mode when Qlib scores are missing, stale, or invalid.

### 2.2 Qlib responsibilities

- Read exported feature tables from Rust.
- Train and compare baseline models.
- Record experiments and artifacts.
- Export score files for replay and shadow mode.
- Manage champion/challenger model versions offline.

### 2.3 Non-goals for v1

- Do not use Qlib as the execution engine.
- Do not use Qlib portfolio backtests as the final trading truth.
- Do not use Qlib RL or online-manager features in the live hot path.
- Do not vendor or fork Qlib into this repository.

## 3. Environment and Tooling

### 3.1 Research runtime

Run Qlib under Linux or WSL. Do not make Windows-native Qlib a required repo
dependency.

### 3.2 Exact setup commands

Use these commands for the research environment:

```bash
cd /path/to/polycli
python3.8 -m venv .venv-qlib
source .venv-qlib/bin/activate
python -m pip install --upgrade pip
pip install pyqlib duckdb pyarrow pandas numpy scikit-learn lightgbm pyyaml
pip install fastparquet matplotlib seaborn jinja2
```

Optional upstream reference checkout:

```bash
git clone https://github.com/microsoft/qlib.git tmp/qlib-reference
```

### 3.3 Rust verification commands

Use these before and after each implementation phase:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
cargo run -- bot --help
```

## 4. Build Order

Implement in this order. Do not change the order.

### Phase 0. Foundation docs and contracts

Deliverables:

- `docs/qlib_integration_plan.md`
- `docs/qlib_task_list.md`
- `docs/research_schema.md`

No code changes in this phase.

### Phase 1. Rust research module scaffold

Create new Rust modules:

- `src/bot/research/mod.rs`
- `src/bot/research/config.rs`
- `src/bot/research/schema.rs`
- `src/bot/research/market_spec.rs`
- `src/bot/research/feature_export.rs`
- `src/bot/research/labeling.rs`
- `src/bot/research/cost_model.rs`
- `src/bot/research/reference_price.rs`
- `src/bot/research/score_loader.rs`
- `src/bot/research/fusion.rs`

Update:

- `src/bot/mod.rs` to export `research`
- `src/commands/bot.rs` to register new CLI commands
- `src/bot/pipeline/mod.rs` for new arg structs and dispatch helpers where
  existing replay/archive commands already live

Definition of done:

- crate compiles with empty placeholder implementations
- new modules have tests for schema serialization and config parsing

### Phase 2. Market-spec normalization

Purpose:

- turn PMXT and Polymarket metadata into a deterministic research key

Implement:

- `CryptoBinaryMarketSpec`
- `MarketFamily`
- `SupportedAsset`
- `SupportedDuration`
- parsing helpers for:
  - `condition_id`
  - `market_slug`
  - asset
  - duration
  - start timestamp
  - end timestamp
  - family

Supported v1 market families:

- `updown_open_close`
- `threshold_at_expiry`

Out of scope in v1:

- narrative crypto markets
- multi-outcome markets
- ambiguous clarified markets

Definition of done:

- replay/archive exports can classify supported markets deterministically
- unsupported markets are skipped with structured reasons

### Phase 3. Research schema and canonical export

Implement the canonical feature export path in Rust first.

Add CLI surfaces:

- `polymarket bot export-features`
- `polymarket bot inspect-features`
- `polymarket bot export-manifest`

Exact CLI definitions:

```text
polymarket bot export-features \
  --input <local-or-remote-parquet> \
  --start <iso8601> \
  --end <iso8601> \
  --asset <btc|eth|sol|xrp|all> \
  --family <updown_open_close|threshold_at_expiry|all> \
  --resolution <1s> \
  --out <path/to/features.parquet> \
  --manifest-out <path/to/manifest.json> \
  --with-labels \
  --with-spot <path-or-url-optional>

polymarket bot inspect-features \
  --input <path/to/features.parquet> \
  --sample 20

polymarket bot export-manifest \
  --input <path/to/features.parquet> \
  --train-start <iso8601> \
  --train-end <iso8601> \
  --valid-start <iso8601> \
  --valid-end <iso8601> \
  --test-start <iso8601> \
  --test-end <iso8601> \
  --out <path/to/manifest.json>
```

Behavior:

- `export-features` is the only command that reads PMXT raw archive files.
- It emits a wide Parquet table following `docs/research_schema.md`.
- It also emits a manifest JSON with dataset version, source files, date ranges,
  and segment definitions.

Definition of done:

- exported parquet can be loaded by DuckDB and pandas
- schema matches `docs/research_schema.md`
- export is deterministic for identical inputs

### Phase 4. Cost model and labels

Implement `src/bot/research/cost_model.rs` and `src/bot/research/labeling.rs`.

Rules:

- use the official Polymarket fee model through one shared cost helper
- express labels in net price-points per share after costs
- never use midpoint-only labels for promotion decisions

Primary labels in v1:

- `label_reachable_yes_15s`
- `label_reachable_yes_30s`
- `label_reachable_yes_45s`
- `label_reachable_no_15s`
- `label_reachable_no_30s`
- `label_reachable_no_45s`
- `label_edge_yes_15s`
- `label_edge_yes_30s`
- `label_edge_yes_45s`
- `label_edge_no_15s`
- `label_edge_no_30s`
- `label_edge_no_45s`
- `label_adverse_yes_30s`
- `label_adverse_no_30s`

Definitions:

- reachable label: `1` if there exists an executable exit bid within the horizon
  that yields positive net edge after fees and slippage
- edge label: maximum net executable edge inside the horizon
- adverse label: worst net drawdown after entering at current ask and marking to
  future bid within the horizon

Definition of done:

- labels are computed only from current and future market data
- no future leakage into features
- unit tests cover fee math and label edge cases

### Phase 5. Score consumption and strategy fusion

Add CLI surfaces:

- `polymarket bot backtest-scores`
- `polymarket bot score-shadow`

Exact CLI definitions:

```text
polymarket bot backtest-scores \
  --input <pmxt-parquet> \
  --scores <path/to/scores.parquet> \
  --strategy <heuristic|qlib|fused> \
  --export <path/to/results.json> \
  --event-log <path/to/logdir>

polymarket bot score-shadow \
  --scores <path/to/scores.parquet> \
  --asset <btc|eth|sol|xrp> \
  --duration <5m|15m|1h> \
  --event-log <path/to/logdir>
```

Implement:

- `ScoreRow`
- `ScoreBundle`
- `ScoreLoader`
- `FusionDecision`
- `fusion.rs` rules

Fusion rules for v1:

- heuristic-only mode ignores Qlib scores
- qlib-only mode uses scores but still obeys Rust risk gates
- fused mode requires:
  - score freshness inside TTL
  - score above configured threshold
  - microstructure/risk constraints to pass

Definition of done:

- replay can compare heuristic, qlib-only, and fused modes on identical data
- missing score rows degrade safely to blocked or heuristic mode by config

### Phase 6. Python research workspace

Create:

- `research/qlib/README.md`
- `research/qlib/requirements.txt`
- `research/qlib/config.py`
- `research/qlib/build_dataset.py`
- `research/qlib/train.py`
- `research/qlib/evaluate.py`
- `research/qlib/export_scores.py`
- `research/qlib/handlers/polymarket_handler.py`
- `research/qlib/tasks/`
- `research/qlib/reports/`

Exact initial commands:

```bash
python -m research.qlib.build_dataset \
  --features data/research/features/btc_5m_v1.parquet \
  --manifest data/research/manifests/btc_5m_v1.json

python -m research.qlib.train \
  --features data/research/features/btc_5m_v1.parquet \
  --manifest data/research/manifests/btc_5m_v1.json \
  --task research/qlib/tasks/btc_5m_yes_30s_lgbm.yaml \
  --out data/research/models/btc_5m_yes_30s_lgbm

python -m research.qlib.evaluate \
  --model-dir data/research/models/btc_5m_yes_30s_lgbm \
  --features data/research/features/btc_5m_v1.parquet \
  --manifest data/research/manifests/btc_5m_v1.json \
  --out data/research/reports/btc_5m_yes_30s_lgbm.json

python -m research.qlib.export_scores \
  --model-dir data/research/models/btc_5m_yes_30s_lgbm \
  --features data/research/features/btc_5m_v1.parquet \
  --out data/research/scores/btc_5m_yes_30s_lgbm.parquet
```

Model order:

- logistic regression baseline
- LightGBM baseline
- shallow MLP only if baseline is positive after replay

Do not implement RL in v1.

### Phase 7. Rolling experiments and promotion policy

Implement rolling windows in Python:

- default split: `30d train / 7d valid / 7d test`
- fallback split: `14d train / 3d valid / 3d test` when data is thin

Promotion rule:

- Qlib model can be promoted to fused shadow mode only if Rust replay shows:
  - positive net expectancy after fees
  - lower or equal max drawdown vs heuristic
  - no fill-quality regression beyond threshold

Promotion artifacts:

- `model.pkl` or model-native artifact
- `metrics.json`
- `feature_manifest.json`
- `score_schema.json`
- `calibration.json`

### Phase 8. Live shadow integration

Live shadow order:

1. heuristic only
2. qlib-only shadow
3. fused shadow
4. dry-run live with fused scores
5. tiny live capital only after acceptance gates

Do not skip steps.

## 5. Repo Structure After Implementation

```text
docs/
  qlib_integration_plan.md
  qlib_task_list.md
  research_schema.md
research/
  qlib/
    README.md
    requirements.txt
    config.py
    build_dataset.py
    train.py
    evaluate.py
    export_scores.py
    handlers/
      polymarket_handler.py
    tasks/
src/
  bot/
    research/
      mod.rs
      config.rs
      schema.rs
      market_spec.rs
      feature_export.rs
      labeling.rs
      cost_model.rs
      reference_price.rs
      score_loader.rs
      fusion.rs
```

## 6. Exact Implementation Rules

- Do not change existing live trading commands until new commands compile.
- Do not remove the current heuristic engine in v1.
- Do not make Python a hard dependency for building the Rust crate.
- All score ingestion must work from local Parquet before adding HTTP serving.
- All replay acceptance metrics must be computed by Rust.
- All timestamps are UTC Unix seconds in Parquet exports.
- All floats in research parquet are `f64`.
- All booleans are stored as `bool` in Rust and `bool`/`uint8` in Parquet.

## 7. Test Plan

### 7.1 Rust unit tests

- market family parsing
- manifest serialization
- feature export schema
- label computation
- fee computation
- score loader validation
- fusion decision logic

### 7.2 Rust integration tests

- export-features produces deterministic parquet
- backtest-scores runs with heuristic-only
- backtest-scores runs with qlib-only
- backtest-scores runs with fused mode
- stale or missing scores are handled safely

### 7.3 Python tests

- dataset load from parquet
- Qlib handler reads expected columns
- train command produces artifact directory
- export_scores emits expected schema

## 8. Acceptance Criteria

The implementation is complete only when all are true:

- docs and schema files exist and match code
- Rust exports a valid research dataset from PMXT archive input
- Qlib training runs on exported data without manual schema patching
- Rust replay can consume Qlib scores in heuristic, qlib-only, and fused modes
- fused mode can be evaluated side-by-side against heuristic mode
- score freshness and fallback behavior are tested
- no live command depends on Python being installed

## 9. Immediate First Implementation Session

Use this exact order for the first coding session:

1. add `src/bot/research/mod.rs` and `schema.rs`
2. add `docs/research_schema.md`
3. add `ExportFeaturesArgs` and `InspectFeaturesArgs`
4. add `export-features` parquet writer
5. add unit tests for schema and export determinism
6. add `research/qlib/requirements.txt`
7. add `build_dataset.py`
8. add `train.py` for logistic regression baseline
9. add `export_scores.py`
10. add `backtest-scores` Rust loader and fused-mode replay
