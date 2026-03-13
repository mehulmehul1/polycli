"""Polymarket Data Handler for Qlib.

This module provides a Qlib-compatible data handler for Polymarket features.
"""

import pandas as pd
import numpy as np
from pathlib import Path
from typing import Optional, List, Union

# Qlib imports (with fallback if not installed)
try:
    from qlib.data.dataset.handler import DataHandlerLP
    QLIB_AVAILABLE = True
except ImportError:
    QLIB_AVAILABLE = False
    DataHandlerLP = object


class PolymarketDataHandler(DataHandlerLP if QLIB_AVAILABLE else object):
    """
    Data handler for Polymarket prediction market data.
    
    Maps Polymarket feature exports to Qlib's expected format:
    - instrument: condition_id (market identifier)
    - datetime: ts (timestamp)
    - features: book state, returns, volatility, etc.
    - labels: reachable, edge, adverse labels
    """
    
    def __init__(
        self,
        instruments: Union[str, List[str]] = "all",
        start_time: Optional[str] = None,
        end_time: Optional[str] = None,
        freq: str = "1s",
        infer_processors: List = None,
        learn_processors: List = None,
        fit_start_time: Optional[str] = None,
        fit_end_time: Optional[str] = None,
        label_cols: List[str] = None,
        feature_cols: List[str] = None,
        data_path: Optional[str] = None,
        **kwargs,
    ):
        """
        Initialize the handler.
        
        Args:
            instruments: "all" or list of condition_ids
            start_time: Start time (format: "YYYY-MM-DD HH:MM:SS" or timestamp)
            end_time: End time
            freq: Data frequency (default: "1s")
            infer_processors: Processors for inference data
            learn_processors: Processors for learning data
            fit_start_time: Fit start time for processors
            fit_end_time: Fit end time for processors
            label_cols: Columns to use as labels
            feature_cols: Columns to use as features
            data_path: Path to parquet data
        """
        if QLIB_AVAILABLE:
            super().__init__(
                instruments=instruments,
                start_time=start_time,
                end_time=end_time,
                freq=freq,
                infer_processors=infer_processors,
                learn_processors=learn_processors,
                fit_start_time=fit_start_time,
                fit_end_time=fit_end_time,
                **kwargs,
            )
        
        self.label_cols = label_cols or ["label_reachable_yes_30s"]
        self.feature_cols = feature_cols
        self.data_path = Path(data_path) if data_path else None
        
        # Store raw data
        self._raw_data = None
    
    def setup_data(self, **kwargs):
        """Setup data from parquet file."""
        if self.data_path is None:
            raise ValueError("data_path must be specified")
        
        # Load parquet
        df = pd.read_parquet(self.data_path)
        
        # Convert timestamp to datetime
        df["datetime"] = pd.to_datetime(df["ts"], unit="s")
        
        # Set instrument as condition_id
        df["instrument"] = df["condition_id"]
        
        # Filter by time range
        if hasattr(self, "start_time") and self.start_time:
            start_ts = pd.Timestamp(self.start_time)
            df = df[df["datetime"] >= start_ts]
        
        if hasattr(self, "end_time") and self.end_time:
            end_ts = pd.Timestamp(self.end_time)
            df = df[df["datetime"] <= end_ts]
        
        # Filter by instruments
        if hasattr(self, "instruments") and self.instruments != "all":
            df = df[df["condition_id"].isin(self.instruments)]
        
        # Set multi-index
        df = df.set_index(["instrument", "datetime"])
        
        self._raw_data = df
        return df
    
    def get_feature_columns(self) -> List[str]:
        """Get list of feature columns."""
        if self.feature_cols:
            return self.feature_cols
        
        if self._raw_data is None:
            return []
        
        # Exclude identity and label columns
        exclude = {
            "schema_version", "condition_id", "market_slug", "instrument",
            "asset", "duration", "market_family", "market_start_ts", "market_end_ts", "ts",
            "datetime", "in_entry_window", "in_exit_only_window",
        }
        exclude.update(self.label_cols)
        
        return [c for c in self._raw_data.columns if c not in exclude]
    
    def fetch(self, selector: Optional[pd.Index] = None) -> pd.DataFrame:
        """Fetch data for given selector."""
        if self._raw_data is None:
            self.setup_data()
        
        if selector is not None:
            return self._raw_data.loc[selector]
        return self._raw_data
    
    def get_labels(self) -> pd.DataFrame:
        """Get label columns."""
        if self._raw_data is None:
            self.setup_data()
        
        available_labels = [c for c in self.label_cols if c in self._raw_data.columns]
        return self._raw_data[available_labels]
    
    def get_features(self) -> pd.DataFrame:
        """Get feature columns."""
        if self._raw_data is None:
            self.setup_data()
        
        feature_cols = self.get_feature_columns()
        available_features = [c for c in feature_cols if c in self._raw_data.columns]
        return self._raw_data[available_features]


def load_polymarket_data(
    parquet_path: str,
    label_col: str = "label_reachable_yes_30s",
    train_ratio: float = 0.7,
    valid_ratio: float = 0.15,
) -> dict:
    """
    Convenience function to load Polymarket data without full Qlib setup.
    
    Args:
        parquet_path: Path to feature parquet
        label_col: Target label column
        train_ratio: Training set ratio
        valid_ratio: Validation set ratio
    
    Returns:
        Dictionary with train/valid/test data
    """
    df = pd.read_parquet(parquet_path)
    
    # Sort by time
    df = df.sort_values("ts")
    
    # Split
    n = len(df)
    train_end = int(n * train_ratio)
    valid_end = int(n * (train_ratio + valid_ratio))
    
    train_df = df.iloc[:train_end]
    valid_df = df.iloc[train_end:valid_end]
    test_df = df.iloc[valid_end:]
    
    # Identify feature columns
    exclude = {
        "schema_version", "condition_id", "market_slug", "instrument",
        "asset", "duration", "market_family", "market_start_ts", "market_end_ts", "ts",
    }
    feature_cols = [c for c in df.columns if c not in exclude and not c.startswith("label_")]
    
    def extract(df):
        X = df[feature_cols].fillna(0).values
        y = df[label_col].values if label_col in df.columns else None
        return X, y
    
    return {
        "train": extract(train_df),
        "valid": extract(valid_df),
        "test": extract(test_df),
        "feature_columns": feature_cols,
    }


if __name__ == "__main__":
    # Example usage
    import sys
    
    if len(sys.argv) > 1:
        data_path = sys.argv[1]
        data = load_polymarket_data(data_path)
        print(f"Loaded data:")
        print(f"  Train: {data['train'][0].shape}")
        print(f"  Valid: {data['valid'][0].shape}")
        print(f"  Test:  {data['test'][0].shape}")
        print(f"  Features: {len(data['feature_columns'])}")
