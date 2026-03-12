# Qlib + PMXT Task List

## Phase 0. Docs and contracts

- [ ] Replace the generic Qlib plan with the strict build-order plan.
- [ ] Add `docs/research_schema.md`.
- [ ] Keep all schema names and command names synchronized across docs.

## Phase 1. Rust research scaffold

- [ ] Create `src/bot/research/mod.rs`.
- [ ] Create `src/bot/research/config.rs`.
- [ ] Create `src/bot/research/schema.rs`.
- [ ] Create `src/bot/research/market_spec.rs`.
- [ ] Create `src/bot/research/feature_export.rs`.
- [ ] Create `src/bot/research/labeling.rs`.
- [ ] Create `src/bot/research/cost_model.rs`.
- [ ] Create `src/bot/research/reference_price.rs`.
- [ ] Create `src/bot/research/score_loader.rs`.
- [ ] Create `src/bot/research/fusion.rs`.
- [ ] Export the module from `src/bot/mod.rs`.

## Phase 2. CLI surface

- [ ] Add `ExportFeatures` to `BotCommand`.
- [ ] Add `InspectFeatures` to `BotCommand`.
- [ ] Add `ExportManifest` to `BotCommand`.
- [ ] Add `BacktestScores` to `BotCommand`.
- [ ] Add `ScoreShadow` to `BotCommand`.
- [ ] Add argument structs in the existing bot/pipeline command surface.
- [ ] Wire dispatch in `src/commands/bot.rs`.
- [ ] Add `--strategy <heuristic|qlib|fused>` where needed.

## Phase 3. Market normalization

- [ ] Implement `SupportedAsset`.
- [ ] Implement `SupportedDuration`.
- [ ] Implement `MarketFamily`.
- [ ] Implement `CryptoBinaryMarketSpec`.
- [ ] Parse market family from slug and metadata.
- [ ] Parse `condition_id`, start time, and end time.
- [ ] Skip unsupported markets with structured reasons.

## Phase 4. Feature export

- [ ] Reuse PMXT archive readers from `src/bot/pipeline/mod.rs`.
- [ ] Build canonical 1-second rows from PMXT raw archive events.
- [ ] Emit feature parquet with schema from `docs/research_schema.md`.
- [ ] Emit manifest JSON next to the parquet output.
- [ ] Add deterministic ordering by `condition_id`, `ts`.
- [ ] Add `inspect-features` sample printer.

## Phase 5. Cost model

- [ ] Implement one shared fee helper in Rust.
- [ ] Express all labels in net price points after fees.
- [ ] Add slippage buffer config to the cost helper.
- [ ] Add tests for low-price and high-price fee scenarios.

## Phase 6. Labels

- [ ] Compute reachable-profit labels for YES.
- [ ] Compute reachable-profit labels for NO.
- [ ] Compute max-edge labels for YES.
- [ ] Compute max-edge labels for NO.
- [ ] Compute adverse-excursion labels.
- [ ] Ensure no feature column reads future values.
- [ ] Add label tests for empty horizon and illiquid edge cases.

## Phase 7. Score loading and fusion

- [ ] Define `ScoreRow`.
- [ ] Define `ScoreBundle`.
- [ ] Load score parquet from disk.
- [ ] Validate model version and score schema version.
- [ ] Validate score TTL and freshness.
- [ ] Implement heuristic-only mode.
- [ ] Implement qlib-only mode.
- [ ] Implement fused mode.
- [ ] Add replay comparison output for all three modes.

## Phase 8. Research workspace

- [ ] Create `research/qlib/README.md`.
- [ ] Create `research/qlib/requirements.txt`.
- [ ] Create `research/qlib/config.py`.
- [ ] Create `research/qlib/build_dataset.py`.
- [ ] Create `research/qlib/train.py`.
- [ ] Create `research/qlib/evaluate.py`.
- [ ] Create `research/qlib/export_scores.py`.
- [ ] Create `research/qlib/handlers/polymarket_handler.py`.
- [ ] Create `research/qlib/tasks/`.

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

- [ ] Add rolling split generation in Python.
- [ ] Default to `30d/7d/7d`.
- [ ] Fall back to `14d/3d/3d` for thin data.
- [ ] Save segment definitions in manifest JSON.
- [ ] Keep train, valid, and test windows strictly time-ordered.

## Phase 12. Shadow integration

- [ ] Add `score-shadow` runner in Rust.
- [ ] Log heuristic, qlib, and fused decisions side by side.
- [ ] Log score freshness and fallback reasons.
- [ ] Keep Qlib optional at runtime.
- [ ] Do not execute live trades from Qlib-only mode in v1.

## Phase 13. Final acceptance

- [ ] `cargo test --all` passes.
- [ ] Python research scripts run on exported parquet without manual edits.
- [ ] Rust replay compares heuristic vs qlib vs fused from one command.
- [ ] Docs match the actual command names and file paths.
- [ ] No live Rust command requires Python.

## Immediate next tasks

- [ ] Implement `src/bot/research/schema.rs`.
- [ ] Implement `ExportFeaturesArgs`.
- [ ] Implement `polymarket bot export-features`.
- [ ] Write `docs/research_schema.md` to match the export.
- [ ] Add one baseline Python dataset loader and trainer.
