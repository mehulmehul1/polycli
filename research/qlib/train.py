"""Train models on exported features."""

import argparse
import pandas as pd
import numpy as np
from pathlib import Path
import joblib
import json
from datetime import datetime

from sklearn.linear_model import LogisticRegression
from sklearn.preprocessing import StandardScaler
from sklearn.pipeline import Pipeline
from sklearn.metrics import accuracy_score, precision_score, recall_score, f1_score, roc_auc_score
import lightgbm as lgb

from config import (
    FEATURES_DIR, MODELS_DIR, FEATURE_COLUMNS, LABEL_COLUMNS,
    DEFAULT_ASSET, DEFAULT_DURATION, DEFAULT_HORIZON
)


def load_data(asset: str, duration: str):
    """Load train/valid/test splits."""
    base_name = f"{asset}_{duration}"
    
    train_path = FEATURES_DIR / f"{base_name}_train.parquet"
    valid_path = FEATURES_DIR / f"{base_name}_valid.parquet"
    test_path = FEATURES_DIR / f"{base_name}_test.parquet"
    
    train_df = pd.read_parquet(train_path) if train_path.exists() else None
    valid_df = pd.read_parquet(valid_path) if valid_path.exists() else None
    test_df = pd.read_parquet(test_path) if test_path.exists() else None
    
    return train_df, valid_df, test_df


def prepare_features(df: pd.DataFrame, target: str, feature_cols: list):
    """Prepare X, y from dataframe."""
    # Filter to available columns
    available_features = [c for c in feature_cols if c in df.columns]
    
    X = df[available_features].values
    y = df[target].values if target in df.columns else None
    
    # Handle missing values
    X = np.nan_to_num(X, nan=0.0)
    
    return X, y, available_features


def train_logistic(X_train, y_train, X_valid=None, y_valid=None):
    """Train logistic regression."""
    model = Pipeline([
        ("scaler", StandardScaler()),
        ("classifier", LogisticRegression(max_iter=1000, C=1.0)),
    ])
    model.fit(X_train, y_train)
    return model


def train_lightgbm(X_train, y_train, X_valid=None, y_valid=None):
    """Train LightGBM classifier."""
    train_data = lgb.Dataset(X_train, label=y_train)
    valid_data = lgb.Dataset(X_valid, label=y_valid) if X_valid is not None else None
    
    params = {
        "objective": "binary",
        "metric": "auc",
        "boosting_type": "gbdt",
        "num_leaves": 31,
        "learning_rate": 0.05,
        "feature_fraction": 0.8,
        "verbose": -1,
    }
    
    model = lgb.train(
        params,
        train_data,
        num_boost_round=500,
        valid_sets=[valid_data] if valid_data else None,
        callbacks=[lgb.early_stopping(50)] if valid_data else None,
    )
    return model


def evaluate_model(model, X, y, model_type="sklearn"):
    """Evaluate model and return metrics."""
    if model_type == "sklearn":
        y_pred = model.predict(X)
        y_proba = model.predict_proba(X)[:, 1]
    else:  # lightgbm
        y_proba = model.predict(X)
        y_pred = (y_proba > 0.5).astype(int)
    
    metrics = {
        "accuracy": accuracy_score(y, y_pred),
        "precision": precision_score(y, y_pred, zero_division=0),
        "recall": recall_score(y, y_pred, zero_division=0),
        "f1": f1_score(y, y_pred, zero_division=0),
        "auc": roc_auc_score(y, y_proba) if len(np.unique(y)) > 1 else 0.5,
    }
    return metrics


def main():
    parser = argparse.ArgumentParser(description="Train models")
    parser.add_argument("--asset", default=DEFAULT_ASSET)
    parser.add_argument("--duration", default=DEFAULT_DURATION)
    parser.add_argument("--target", default="label_reachable_yes_30s", choices=LABEL_COLUMNS)
    parser.add_argument("--model", default="logistic", choices=["logistic", "lightgbm"])
    parser.add_argument("--output", default=None)
    args = parser.parse_args()
    
    # Load data
    train_df, valid_df, test_df = load_data(args.asset, args.duration)
    
    if train_df is None:
        print(f"No training data found for {args.asset}_{args.duration}")
        return
    
    # Prepare features
    X_train, y_train, feature_cols = prepare_features(train_df, args.target, FEATURE_COLUMNS)
    X_valid, y_valid, _ = prepare_features(valid_df, args.target, FEATURE_COLUMNS) if valid_df is not None else (None, None, None)
    X_test, y_test, _ = prepare_features(test_df, args.target, FEATURE_COLUMNS) if test_df is not None else (None, None, None)
    
    print(f"Training on {len(X_train)} samples with {len(feature_cols)} features")
    print(f"Target: {args.target} (positive rate: {y_train.mean():.2%})")
    
    # Train model
    if args.model == "logistic":
        model = train_logistic(X_train, y_train, X_valid, y_valid)
        model_type = "sklearn"
    else:
        model = train_lightgbm(X_train, y_train, X_valid, y_valid)
        model_type = "lightgbm"
    
    # Evaluate
    train_metrics = evaluate_model(model, X_train, y_train, model_type)
    print(f"\nTrain metrics: {train_metrics}")
    
    if X_valid is not None and y_valid is not None:
        valid_metrics = evaluate_model(model, X_valid, y_valid, model_type)
        print(f"Valid metrics: {valid_metrics}")
    
    if X_test is not None and y_test is not None:
        test_metrics = evaluate_model(model, X_test, y_test, model_type)
        print(f"Test metrics: {test_metrics}")
    
    # Save model
    output_path = Path(args.output) if args.output else MODELS_DIR / f"{args.model}_{args.target.replace('label_', '')}.pkl"
    output_path.parent.mkdir(parents=True, exist_ok=True)
    
    model_data = {
        "model": model,
        "model_type": model_type,
        "feature_columns": feature_cols,
        "target": args.target,
        "train_metrics": train_metrics,
        "valid_metrics": valid_metrics if X_valid is not None else None,
        "test_metrics": test_metrics if X_test is not None else None,
        "trained_at": datetime.utcnow().isoformat(),
    }
    
    joblib.dump(model_data, output_path)
    print(f"\nSaved model to {output_path}")


if __name__ == "__main__":
    main()
