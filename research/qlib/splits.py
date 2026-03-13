"""Rolling train/valid/test split generation."""

from typing import List, Dict, Tuple, Optional
import pandas as pd


def generate_rolling_splits(
    start_ts: int,
    end_ts: int,
    train_days: int = 30,
    valid_days: int = 7,
    test_days: int = 7,
    step_days: Optional[int] = None,
) -> List[Dict[str, int]]:
    """
    Generate rolling train/valid/test splits.
    
    Args:
        start_ts: Start timestamp (seconds)
        end_ts: End timestamp (seconds)
        train_days: Training window in days
        valid_days: Validation window in days
        test_days: Test window in days
        step_days: Step size for rolling (default: test_days)
    
    Returns:
        List of split dictionaries with train/valid/test timestamps
    """
    day_seconds = 24 * 3600
    train_seconds = train_days * day_seconds
    valid_seconds = valid_days * day_seconds
    test_seconds = test_days * day_seconds
    step_seconds = (step_days or test_days) * day_seconds
    
    splits = []
    current_start = start_ts
    
    while True:
        train_start = current_start
        train_end = train_start + train_seconds
        valid_start = train_end
        valid_end = valid_start + valid_seconds
        test_start = valid_end
        test_end = test_start + test_seconds
        
        if test_end > end_ts:
            break
        
        splits.append({
            "train_start_ts": train_start,
            "train_end_ts": train_end,
            "valid_start_ts": valid_start,
            "valid_end_ts": valid_end,
            "test_start_ts": test_start,
            "test_end_ts": test_end,
        })
        
        current_start += step_seconds
    
    return splits


def fallback_splits(thin_data: bool = False) -> Tuple[int, int, int]:
    """
    Get split sizes based on data availability.
    
    Args:
        thin_data: If True, use smaller windows
    
    Returns:
        Tuple of (train_days, valid_days, test_days)
    """
    if thin_data:
        return (14, 3, 3)
    return (30, 7, 7)


def auto_splits_from_data(
    df: pd.DataFrame,
    ts_col: str = "ts",
    min_train_samples: int = 1000,
) -> Tuple[List[Dict[str, int]], str]:
    """
    Automatically determine split strategy from data.
    
    Args:
        df: Dataframe with timestamp column
        ts_col: Name of timestamp column
        min_train_samples: Minimum samples required for training
    
    Returns:
        Tuple of (splits, strategy_name)
    """
    if ts_col not in df.columns:
        raise ValueError(f"Timestamp column '{ts_col}' not found")
    
    start_ts = int(df[ts_col].min())
    end_ts = int(df[ts_col].max())
    total_samples = len(df)
    
    # Determine if data is thin
    duration_days = (end_ts - start_ts) / (24 * 3600)
    samples_per_day = total_samples / max(duration_days, 1)
    
    thin_data = samples_per_day < 500 or duration_days < 45
    
    train_days, valid_days, test_days = fallback_splits(thin_data)
    
    splits = generate_rolling_splits(
        start_ts, end_ts,
        train_days, valid_days, test_days
    )
    
    strategy = "thin" if thin_data else "standard"
    return splits, strategy


if __name__ == "__main__":
    # Example usage
    import time
    
    now = int(time.time())
    start = now - 60 * 24 * 3600  # 60 days ago
    
    splits = generate_rolling_splits(start, now)
    print(f"Generated {len(splits)} rolling splits")
    
    for i, split in enumerate(splits[:3]):
        print(f"\nSplit {i + 1}:")
        for k, v in split.items():
            print(f"  {k}: {v}")
