# Polymarket Research with Qlib

This directory contains the Python research workspace for training ML models on exported Polymarket features.

## Setup

```bash
cd research/qlib
pip install -r requirements.txt
```

## Workflow

1. **Export features from Rust CLI:**
   ```bash
   polymarket bot export-features --input data/pmxt/archive.parquet --out data/research/features/btc_5m.parquet
   ```

2. **Build Qlib dataset:**
   ```bash
   python build_dataset.py --input data/research/features/btc_5m.parquet
   ```

3. **Train models:**
   ```bash
   python train.py --target label_reachable_yes_30s
   ```

4. **Evaluate:**
   ```bash
   python evaluate.py --model models/logistic_yes_30s.pkl
   ```

5. **Export scores for Rust:**
   ```bash
   python export_scores.py --model models/logistic_yes_30s.pkl --out data/research/scores/btc_5m_scores.parquet
   ```

## Directory Structure

```
research/
├── qlib/
│   ├── config.py           # Paths and settings
│   ├── build_dataset.py    # Load parquet → Qlib
│   ├── train.py            # Train models
│   ├── evaluate.py         # Model evaluation
│   ├── export_scores.py    # Export score parquet
│   ├── splits.py           # Rolling split generation
│   ├── handlers/
│   │   ├── __init__.py
│   │   └── polymarket_handler.py
│   └── tasks/              # YAML task configs
├── data/
│   ├── features/           # Rust feature exports
│   ├── scores/             # Model score exports
│   └── manifests/          # Dataset manifests
└── models/                 # Trained model files
```

## Labels

The following labels are available for training:

- `label_reachable_yes_30s` - Is YES profitable within 30s?
- `label_reachable_no_30s` - Is NO profitable within 30s?
- `label_edge_yes_30s` - Max YES edge within 30s
- `label_edge_no_30s` - Max NO edge within 30s
- `label_adverse_yes_30s` - Worst YES drawdown
- `label_adverse_no_30s` - Worst NO drawdown

## Schemas

See `docs/research_schema.md` for exact column definitions.
