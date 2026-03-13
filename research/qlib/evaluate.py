"""Evaluate trained models."""

import argparse
import pandas as pd
import numpy as np
from pathlib import Path
import joblib
import json

from sklearn.metrics import (
    accuracy_score, precision_score, recall_score, f1_score,
    roc_auc_score, confusion_matrix, classification_report
)

from config import FEATURES_DIR, MODELS_DIR, FEATURE_COLUMNS


def main():
    parser = argparse.ArgumentParser(description="Evaluate model")
    parser.add_argument("--model", required=True, help="Path to model file")
    parser.add_argument("--data", default=None, help="Test data path (optional)")
    parser.add_argument("--output", default=None, help="Output metrics path")
    args = parser.parse_args()
    
    # Load model
    model_path = Path(args.model)
    model_data = joblib.load(model_path)
    
    model = model_data["model"]
    model_type = model_data["model_type"]
    feature_cols = model_data["feature_columns"]
    target = model_data["target"]
    
    print(f"Model: {model_path.name}")
    print(f"Type: {model_type}")
    print(f"Target: {target}")
    print(f"Features: {len(feature_cols)}")
    
    # Load test data
    if args.data:
        test_df = pd.read_parquet(args.data)
    else:
        # Try to find test split
        asset = "btc"  # Default
        duration = "5m"
        test_path = FEATURES_DIR / f"{asset}_{duration}_test.parquet"
        if not test_path.exists():
            print("No test data found")
            return
        test_df = pd.read_parquet(test_path)
    
    # Prepare features
    available_features = [c for c in feature_cols if c in test_df.columns]
    X_test = test_df[available_features].values
    X_test = np.nan_to_num(X_test, nan=0.0)
    
    if target not in test_df.columns:
        print(f"Target {target} not in data")
        return
    
    y_test = test_df[target].values
    
    # Predict
    if model_type == "sklearn":
        y_pred = model.predict(X_test)
        y_proba = model.predict_proba(X_test)[:, 1]
    else:  # lightgbm
        y_proba = model.predict(X_test)
        y_pred = (y_proba > 0.5).astype(int)
    
    # Metrics
    metrics = {
        "accuracy": accuracy_score(y_test, y_pred),
        "precision": precision_score(y_test, y_pred, zero_division=0),
        "recall": recall_score(y_test, y_pred, zero_division=0),
        "f1": f1_score(y_test, y_pred, zero_division=0),
        "auc": roc_auc_score(y_test, y_proba) if len(np.unique(y_test)) > 1 else 0.5,
        "n_samples": len(y_test),
        "positive_rate": float(y_test.mean()),
    }
    
    print("\n" + "=" * 50)
    print("Test Set Evaluation")
    print("=" * 50)
    for k, v in metrics.items():
        if isinstance(v, float):
            print(f"{k}: {v:.4f}")
        else:
            print(f"{k}: {v}")
    
    print("\nConfusion Matrix:")
    print(confusion_matrix(y_test, y_pred))
    
    print("\nClassification Report:")
    print(classification_report(y_test, y_pred, zero_division=0))
    
    # Score distribution
    print("\nScore Distribution:")
    print(f"  min:    {y_proba.min():.4f}")
    print(f"  25%:    {np.percentile(y_proba, 25):.4f}")
    print(f"  median: {np.percentile(y_proba, 50):.4f}")
    print(f"  75%:    {np.percentile(y_proba, 75):.4f}")
    print(f"  max:    {y_proba.max():.4f}")
    
    # Save metrics
    if args.output:
        output_path = Path(args.output)
        with open(output_path, "w") as f:
            json.dump(metrics, f, indent=2)
        print(f"\nSaved metrics to {output_path}")


if __name__ == "__main__":
    main()
