"""Export model scores for Rust consumption."""

import argparse
import pandas as pd
import numpy as np
from pathlib import Path
import joblib
import pyarrow as pa
import pyarrow.parquet as pq
from datetime import datetime, timedelta

from config import FEATURES_DIR, SCORES_DIR, MODELS_DIR, FRESHNESS_TTL_SECONDS


def main():
    parser = argparse.ArgumentParser(description="Export scores")
    parser.add_argument("--model", required=True, help="Path to model file")
    parser.add_argument("--data", default=None, help="Input data path")
    parser.add_argument("--output", default=None, help="Output scores path")
    parser.add_argument("--freshness-ttl", type=int, default=FRESHNESS_TTL_SECONDS)
    args = parser.parse_args()
    
    # Load model
    model_path = Path(args.model)
    model_data = joblib.load(model_path)
    
    model = model_data["model"]
    model_type = model_data["model_type"]
    feature_cols = model_data["feature_columns"]
    target = model_data["target"]
    
    # Load data
    if args.data:
        df = pd.read_parquet(args.data)
    else:
        # Use all features
        df = pd.read_parquet(FEATURES_DIR / "btc_5m.parquet")
    
    print(f"Loaded {len(df)} rows")
    
    # Prepare features
    available_features = [c for c in feature_cols if c in df.columns]
    X = df[available_features].values
    X = np.nan_to_num(X, nan=0.0)
    
    # Predict
    if model_type == "sklearn":
        scores = model.predict_proba(X)[:, 1]
    else:  # lightgbm
        scores = model.predict(X)
    
    # Build score dataframe
    # Determine direction from target
    is_yes = "yes" in target.lower()
    horizon = "30s"  # Default
    
    # Extract horizon from target
    for h in ["15s", "30s", "45s"]:
        if h in target:
            horizon = h
            break
    
    score_df = pd.DataFrame({
        "schema_version": "scores_v1",
        "condition_id": df["condition_id"],
        "ts": df["ts"],
        "model_version": model_path.stem,
        f"score_{'yes' if is_yes else 'no'}_{horizon}": scores,
        "fresh_until_ts": df["ts"] + args.freshness_ttl,
    })
    
    # Fill missing score columns with defaults
    for direction in ["yes", "no"]:
        for h in ["15s", "30s", "45s"]:
            col = f"score_{direction}_{h}"
            if col not in score_df.columns:
                score_df[col] = 0.5  # Neutral score
    
    # Add risk scores (placeholder)
    score_df["risk_yes_30s"] = 0.1
    score_df["risk_no_30s"] = 0.1
    
    # Reorder columns
    columns = [
        "schema_version", "condition_id", "ts", "model_version",
        "score_yes_15s", "score_yes_30s", "score_yes_45s",
        "score_no_15s", "score_no_30s", "score_no_45s",
        "risk_yes_30s", "risk_no_30s", "fresh_until_ts",
    ]
    score_df = score_df[columns]
    
    # Save
    output_path = Path(args.output) if args.output else SCORES_DIR / f"{model_path.stem}_scores.parquet"
    output_path.parent.mkdir(parents=True, exist_ok=True)
    
    score_df.to_parquet(output_path, index=False)
    print(f"Exported {len(score_df)} scores to {output_path}")
    
    # Summary
    print(f"\nScore summary for {col}:")
    print(f"  mean:   {scores.mean():.4f}")
    print(f"  std:    {scores.std():.4f}")
    print(f"  min:    {scores.min():.4f}")
    print(f"  max:    {scores.max():.4f}")


if __name__ == "__main__":
    main()
