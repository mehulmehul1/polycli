"""Polymarket Research Configuration"""

from pathlib import Path

# Paths
RESEARCH_ROOT = Path(__file__).parent.parent
QLIB_ROOT = Path(__file__).parent
FEATURES_DIR = RESEARCH_ROOT / "data" / "features"
SCORES_DIR = RESEARCH_ROOT / "data" / "scores"
MANIFESTS_DIR = RESEARCH_ROOT / "data" / "manifests"
MODELS_DIR = RESEARCH_ROOT / "models"

# Ensure directories exist
for d in [FEATURES_DIR, SCORES_DIR, MANIFESTS_DIR, MODELS_DIR]:
    d.mkdir(parents=True, exist_ok=True)

# Default settings
DEFAULT_ASSET = "btc"
DEFAULT_DURATION = "5m"
DEFAULT_HORIZON = 30

# Model settings
SCORE_THRESHOLD = 0.3
FRESHNESS_TTL_SECONDS = 300

# Label columns
LABEL_COLUMNS = [
    "label_reachable_yes_15s",
    "label_reachable_yes_30s",
    "label_reachable_yes_45s",
    "label_reachable_no_15s",
    "label_reachable_no_30s",
    "label_reachable_no_45s",
    "label_edge_yes_30s",
    "label_edge_no_30s",
    "label_adverse_yes_30s",
    "label_adverse_no_30s",
]

# Feature columns (excluding identity and labels)
FEATURE_COLUMNS = [
    "yes_bid", "yes_ask", "no_bid", "no_ask",
    "yes_mid", "no_mid", "yes_spread", "no_spread",
    "book_sum", "book_gap",
    "yes_mid_ret_1s", "yes_mid_ret_5s", "yes_mid_ret_15s", "yes_mid_ret_30s",
    "no_mid_ret_1s", "no_mid_ret_5s", "no_mid_ret_15s", "no_mid_ret_30s",
    "book_gap_z_30s", "yes_spread_ema_15s", "no_spread_ema_15s",
    "realized_vol_15s", "realized_vol_30s", "realized_vol_60s",
    "age_s", "time_remaining_s",
]
