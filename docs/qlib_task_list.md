# Qlib + PMXT Task List

## Phase 0. Docs and contracts

- [x] Replace the generic Qlib plan with the strict build-order plan.
- [x] Add `docs/research_schema.md`.
- [x] Keep all schema names and command names synchronized across docs.

## Phase 0.5. Design docs (strategy, fair-value, execution)

- [x] Add `docs/strategy_engine_spec.md`.
- [x] Add `docs/fair_value_model.md`.
- [x] Add `docs/execution_spec.md`.
- [x] Review and approve all design docs before implementation.
- [x] Update `docs/qlib_integration_plan.md` to reference new design docs.

## Phase 1. Rust research scaffold

- [x] Create `src/bot/research/mod.rs`.
- [x] Create `src/bot/research/config.rs`.
- [x] Create `src/bot/research/schema.rs`.
- [x] Create `src/bot/research/market_spec.rs`.
- [x] Create `src/bot/research/feature_export.rs`.
- [x] Create `src/bot/research/labeling.rs`.
- [x] Create `src/bot/research/cost_model.rs`.
- [ ] Create `src/bot/research/reference_price.rs`.
- [x] Create `src/bot/research/score_loader.rs`.
- [x] Create `src/bot/research/fusion.rs`.
- [x] Export the module from `src/bot/mod.rs`.

## Phase 1b. Strategy Engine (NEW)

- [x] Create `src/bot/strategy/mod.rs` with StrategyEngine trait.
- [x] Create `src/bot/strategy/types.rs` with Direction, Confidence, StrategyDecision.
- [x] Create `src/bot/strategy/heuristic.rs` migrating from signal.rs.
- [x] Create `src/bot/strategy/risk.rs` with RiskGate.
- [x] Create `src/bot/pricing/mod.rs`.
- [x] Create `src/bot/pricing/fair_value.rs` with digital option approximation.
- [x] Create `src/bot/pricing/volatility.rs` with realized vol calculator.

## Phase 2. CLI surface

- [x] Add `ExportFeatures` to `BotCommand`.
- [x] Add `InspectFeatures` to `BotCommand`.
- [x] Add `ExportManifest` to `BotCommand`.
- [x] Add `BacktestScores` to `BotCommand`.
- [x] Add `ScoreShadow` to `BotCommand`.
- [x] Add argument structs in the existing bot/pipeline command surface.
- [x] Wire dispatch in `src/commands/bot.rs`.
- [x] Add `--strategy <heuristic|qlib|fused>` where needed.

## Phase 3. Market normalization

- [x] Implement `SupportedAsset`.
- [x] Implement `SupportedDuration`.
- [x] Implement `MarketFamily`.
- [x] Implement `CryptoBinaryMarketSpec`.
- [x] Parse market family from slug and metadata.
- [x] Parse `condition_id`, start time, and end time.
- [x] Skip unsupported markets with structured reasons.

## Phase 4. Feature export

- [x] Reuse PMXT archive readers from `src/bot/pipeline/mod.rs`.
- [x] Build canonical 1-second rows from PMXT raw archive events.
- [x] Emit feature parquet with schema from `docs/research_schema.md`.
- [x] Emit manifest JSON next to the parquet output.
- [x] Add deterministic ordering by `condition_id`, `ts`.
- [x] Add `inspect-features` sample printer.

## Phase 5. Cost model

- [x] Implement one shared fee helper in Rust.
- [x] Express all labels in net price points after fees.
- [x] Add slippage buffer config to the cost helper.
- [x] Add tests for low-price and high-price fee scenarios.

## Phase 6. Labels

- [x] Compute reachable-profit labels for YES.
- [x] Compute reachable-profit labels for NO.
- [x] Compute max-edge labels for YES.
- [x] Compute max-edge labels for NO.
- [x] Compute adverse-excursion labels.
- [x] Ensure no feature column reads future values.
- [x] Add label tests for empty horizon and illiquid edge cases.

## Phase 7. Score loading and fusion

- [x] Define `ScoreRow`.
- [x] Define `ScoreBundle`.
- [x] Load score parquet from disk.
- [x] Validate model version and score schema version.
- [x] Validate score TTL and freshness.
- [x] Implement heuristic-only mode.
- [x] Implement qlib-only mode.
- [x] Implement fused mode.
- [x] Add replay comparison output for all three modes.

## Phase 8. Research workspace

- [x] Create `research/qlib/README.md`.
- [x] Create `research/qlib/requirements.txt`.
- [x] Create `research/qlib/config.py`.
- [x] Create `research/qlib/build_dataset.py`.
- [x] Create `research/qlib/train.py`.
- [x] Create `research/qlib/evaluate.py`.
- [x] Create `research/qlib/export_scores.py`.
- [x] Create `research/qlib/handlers/polymarket_handler.py`.
- [x] Create `research/qlib/tasks/`.
- [x] Create `research/qlib/splits.py` for rolling validation.

## Phase 9. Baseline models

- [ ] Train logistic regression on `label_reachable_yes_30s`.
- [ ] Train logistic regression on `label_reachable_no_30s`.
- [ ] Train LightGBM on the same targets.
- [ ] Export model metrics to JSON.
- [ ] Export score parquet with the agreed schema.
- [ ] Compare model score distribution against the heuristic signal frequency.

## Phase 10. Replay validation

- [ ] Run heuristic-only replay on one BTC 5m sample.
- [ ] Run qlib-only replay on the same sample.
- [ ] Run fused replay on the same sample.
- [ ] Confirm replay uses Rust fee and fill logic.
- [ ] Confirm missing scores fail safe.
- [ ] Confirm stale scores fail safe.
- [ ] Confirm score timestamps align with replay timestamps.

## Phase 11. Rolling validation

- [x] Add rolling split generation in Python.
- [x] Default to `30d/7d/7d`.
- [x] Fall back to `14d/3d/3d` for thin data.
- [x] Save segment definitions in manifest JSON.
- [x] Keep train, valid, and test windows strictly time-ordered.

## Phase 12. Shadow integration

- [x] Add `score-shadow` runner in Rust.
- [x] Log heuristic, qlib, and fused decisions side by side.
- [x] Log score freshness and fallback reasons.
- [x] Keep Qlib optional at runtime.
- [x] Do not execute live trades from Qlib-only mode in v1.

## Phase 13. Final acceptance

- [ ] `cargo test --all` passes.
- [ ] Python research scripts run on exported parquet without manual edits.
- [ ] Rust replay compares heuristic vs qlib vs fused from one command.
- [ ] Docs match the actual command names and file paths.
- [ ] No live Rust command requires Python.

## Immediate next tasks

- [ ] Run `cargo check` and fix any remaining compilation errors
- [ ] Wire `strategy_runner.rs` to use new StrategyEngine trait
- [ ] Update `shadow.rs` for fusion logging
- [ ] Train baseline models with Python scripts
