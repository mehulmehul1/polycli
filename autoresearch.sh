#!/bin/bash
# autoresearch.sh — Continuous backtest loop for strategy optimization
#
# Usage: ./autoresearch.sh <strategy> [iterations]
# Example: ./autoresearch.sh scalper 50
#
# This script:
# 1. Builds the project
# 2. Runs backtest-pmxt on recordings/
# 3. Computes fitness score from results JSON
# 4. Logs to results.tsv
# 5. Loops for N iterations

STRATEGY=${1:-scalper}
ITERATIONS=${2:-100}
RESULTS_FILE="results_${STRATEGY}.tsv"
RECORDINGS_DIR="recordings"
CAPITAL=5
SIZE=1
MIN_EDGE=0.05

echo "=== Autoresearch: $STRATEGY ==="
echo "Iterations: $ITERATIONS"
echo "Recordings: $RECORDINGS_DIR"
echo "Results: $RESULTS_FILE"
echo "================================"

# Create results file with header
if [ ! -f "$RESULTS_FILE" ]; then
    echo -e "timestamp\tgit_hash\tstrategy\ttrades\twin_rate\tavg_win\tavg_loss\tprofit_factor\tnet_pnl_pct\tmax_drawdown\tfitness" > "$RESULTS_FILE"
fi

# Build once
echo "[BUILD] Building release binary..."
cargo build --release 2>/dev/null
if [ $? -ne 0 ]; then
    echo "[ERROR] Build failed"
    exit 1
fi

for i in $(seq 1 $ITERATIONS); do
    echo ""
    echo "[ITERATION $i/$ITERATIONS] $(date '+%Y-%m-%d %H:%M:%S')"
    
    # Run backtest
    RESULTS_JSON="results_${STRATEGY}_latest.json"
    cargo run --release -q -- bot backtest-pmxt \
        --input-dir "$RECORDINGS_DIR" \
        --strategy "$STRATEGY" \
        --capital "$CAPITAL" \
        --size "$SIZE" \
        --min-edge "$MIN_EDGE" \
        --export "$RESULTS_JSON" 2>/dev/null
    
    if [ ! -f "$RESULTS_JSON" ]; then
        echo "[WARN] No results file produced, skipping"
        continue
    fi
    
    # Extract metrics from JSON
    TRADES=$(python3 -c "import json; d=json.load(open('$RESULTS_JSON')); print(d.get('trades_taken', 0))")
    WIN_RATE=$(python3 -c "import json; d=json.load(open('$RESULTS_JSON')); print(d.get('win_rate', 0))")
    AVG_WIN=$(python3 -c "import json; d=json.load(open('$RESULTS_JSON')); print(d.get('avg_win', 0))")
    AVG_LOSS=$(python3 -c "import json; d=json.load(open('$RESULTS_JSON')); print(d.get('avg_loss', 0))")
    PROFIT_FACTOR=$(python3 -c "import json; d=json.load(open('$RESULTS_JSON')); print(d.get('profit_factor', 0))")
    NET_PNL=$(python3 -c "import json; d=json.load(open('$RESULTS_JSON')); print(d.get('total_pnl_pct', 0))")
    MAX_DD=$(python3 -c "import json; d=json.load(open('$RESULTS_JSON')); print(d.get('max_drawdown', 0))")
    
    # Compute fitness
    FITNESS=$(python3 -c "
wf = float('$WIN_RATE') / 100.0
pf = min(float('$PROFIT_FACTOR'), 10.0)
np = float('$NET_PNL') / 100.0
fitness = (pf * 0.35) + (wf * 0.25) + (np * 0.25)
print(f'{fitness:.4f}')
")
    
    GIT_HASH=$(git rev-parse --short HEAD)
    TIMESTAMP=$(date '+%Y-%m-%d %H:%M:%S')
    
    # Log to TSV
    echo -e "${TIMESTAMP}\t${GIT_HASH}\t${STRATEGY}\t${TRADES}\t${WIN_RATE}\t${AVG_WIN}\t${AVG_LOSS}\t${PROFIT_FACTOR}\t${NET_PNL}\t${MAX_DD}\t${FITNESS}" >> "$RESULTS_FILE"
    
    echo "[RESULT] Trades: $TRADES | Win Rate: ${WIN_RATE}% | Fitness: $FITNESS"
    
    # Wait between iterations (allow new recordings to accumulate)
    if [ $i -lt $ITERATIONS ]; then
        echo "[WAIT] Sleeping 60s before next iteration..."
        sleep 60
    fi
done

echo ""
echo "=== Autoresearch Complete ==="
echo "Results saved to: $RESULTS_FILE"
echo ""
echo "Best fitness:"
tail -n +2 "$RESULTS_FILE" | sort -t$'\t' -k11 -rn | head -5
