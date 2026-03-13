"""Build Qlib dataset from Rust feature exports."""

import argparse
import pandas as pd
import pyarrow.parquet as pq
from pathlib import Path
import json

from config import FEATURES_DIR, MANIFESTS_DIR, LABEL_COLUMNS, FEATURE_COLUMNS


def load_features(parquet_path: Path) -> pd.DataFrame:
    """Load features from Rust-exported parquet."""
    df = pd.read_parquet(parquet_path)
    print(f"Loaded {len(df)} rows from {parquet_path}")
    return df


def validate_schema(df: pd.DataFrame) -> bool:
    """Validate feature schema."""
    required = ["condition_id", "ts", "schema_version"]
    missing = [c for c in required if c not in df.columns]
    if missing:
        print(f"Missing required columns: {missing}")
        return False
    
    version = df["schema_version"].iloc[0] if len(df) > 0 else None
    if version != "features_v1":
        print(f"Unexpected schema version: {version}")
        return False
    
    return True


def prepare_for_qlib(df: pd.DataFrame) -> pd.DataFrame:
    """Prepare dataframe for Qlib format."""
    # Set instrument and datetime
    df = df.copy()
    df["instrument"] = df["condition_id"]
    df["datetime"] = pd.to_datetime(df["ts"], unit="s")
    df = df.set_index(["instrument", "datetime"])
    return df


def create_splits(df: pd.DataFrame, train_pct: float = 0.7, valid_pct: float = 0.15):
    """Create train/valid/test splits by time."""
    min_ts = df["ts"].min()
    max_ts = df["ts"].max()
    total_duration = max_ts - min_ts
    
    train_end = min_ts + int(total_duration * train_pct)
    valid_end = min_ts + int(total_duration * (train_pct + valid_pct))
    
    train_df = df[df["ts"] <= train_end]
    valid_df = df[(df["ts"] > train_end) & (df["ts"] <= valid_end)]
    test_df = df[df["ts"] > valid_end]
    
    print(f"Train: {len(train_df)} rows ({min_ts} - {train_end})")
    print(f"Valid: {len(valid_df)} rows ({train_end} - {valid_end})")
    print(f"Test:  {len(test_df)} rows ({valid_end} - {max_ts})")
    
    return train_df, valid_df, test_df, {
        "train_start_ts": int(min_ts),
        "train_end_ts": int(train_end),
        "valid_start_ts": int(train_end),
        "valid_end_ts": int(valid_end),
        "test_start_ts": int(valid_end),
        "test_end_ts": int(max_ts),
    }


def save_manifest(parquet_path: Path, df: pd.DataFrame, splits: dict, out_path: Path):
    """Save dataset manifest."""
    manifest = {
        "schema_version": "manifest_v1",
        "features_path": str(parquet_path),
        "asset": df["asset"].iloc[0] if len(df) > 0 else "unknown",
        "duration": df["duration"].iloc[0] if len(df) > 0 else "5m",
        "market_family": df["market_family"].iloc[0] if len(df) > 0 else "unknown",
        "start_ts": int(df["ts"].min()),
        "end_ts": int(df["ts"].max()),
        "resolution": "1s",
        "source_inputs": [str(parquet_path)],
        **splits,
        "label_columns": LABEL_COLUMNS,
        "feature_columns": FEATURE_COLUMNS,
    }
    
    with open(out_path, "w") as f:
        json.dump(manifest, f, indent=2)
    print(f"Saved manifest to {out_path}")


def main():
    parser = argparse.ArgumentParser(description="Build Qlib dataset")
    parser.add_argument("--input", required=True, help="Input parquet path")
    parser.add_argument("--output", default=None, help="Output parquet path")
    parser.add_argument("--manifest", default=None, help="Manifest output path")
    args = parser.parse_args()
    
    input_path = Path(args.input)
    output_path = Path(args.output) if args.output else FEATURES_DIR / input_path.name
    manifest_path = Path(args.manifest) if args.manifest else MANIFESTS_DIR / f"{input_path.stem}_manifest.json"
    
    # Load and validate
    df = load_features(input_path)
    if not validate_schema(df):
        raise ValueError("Invalid feature schema")
    
    # Create splits
    train_df, valid_df, test_df, splits = create_splits(df)
    
    # Save processed data
    output_path.parent.mkdir(parents=True, exist_ok=True)
    df.to_parquet(output_path, index=False)
    print(f"Saved processed features to {output_path}")
    
    # Save manifest
    save_manifest(input_path, df, splits, manifest_path)
    
    # Save splits
    for name, split_df in [("train", train_df), ("valid", valid_df), ("test", test_df)]:
        split_path = output_path.parent / f"{output_path.stem}_{name}.parquet"
        split_df.to_parquet(split_path, index=False)
        print(f"Saved {name} split: {len(split_df)} rows")


if __name__ == "__main__":
    main()
