#!/usr/bin/env bash
# bench.sh — reproducible timing benchmark for fightfake-toolkit
#
# Usage:
#   ./bench.sh [VIDEO] [RUNS]
#
# Defaults to testdata/videos/input/bank-robbery-original.mp4, 3 runs.
# Prints per-phase timings for each run, then averages.
#
# Requirements: the release binary must already be built.
#   cargo build -p fightfake-cli --release

set -euo pipefail

VIDEO="${1:-testdata/videos/input/bank-robbery-original.mp4}"
RUNS="${2:-3}"
BINARY="./target/release/fightfake"

if [[ ! -f "$BINARY" ]]; then
  echo "error: $BINARY not found — run: cargo build -p fightfake-cli --release" >&2
  exit 1
fi
if [[ ! -f "$VIDEO" ]]; then
  echo "error: video file not found: $VIDEO" >&2
  exit 1
fi

echo "benchmark: $VIDEO ($RUNS runs)"
echo "binary   : $(file "$BINARY" | cut -d: -f2 | xargs)"
echo ""

# Temp dir for outputs (excluded from git via .gitignore)
OUTDIR="out/bench"
mkdir -p "$OUTDIR"

# Arrays for each phase (seconds, accumulated)
declare -a total_decode=()
declare -a total_tile=()
declare -a total_hash=()
declare -a total_prove=()
declare -a total_encode=()
declare -a total_sign=()
declare -a total_wall=()

for run in $(seq 1 "$RUNS"); do
  echo "=== run $run / $RUNS ==="
  output=$("$BINARY" prove-edit \
    --input   "$VIDEO" \
    --gadget  brightness \
    --gadget-param 416 \
    --out-dir "$OUTDIR" 2>&1)

  echo "$output" | grep -E "^\[workflow\]|^┌|^│|^└|^├" | head -20

  # Extract timings (lines like: │ ffmpeg decode    │  0.54s │)
  decode=$(echo "$output"  | grep "ffmpeg decode"            | grep -oE '[0-9]+\.[0-9]+' | head -1)
  tile=$(echo "$output"    | grep "macroblock tiling"        | grep -oE '[0-9]+\.[0-9]+' | head -1)
  hash=$(echo "$output"    | grep "edit + hashing"           | grep -oE '[0-9]+\.[0-9]+' | head -1)
  prove=$(echo "$output"   | grep "ZK proving"               | grep -oE '[0-9]+\.[0-9]+' | head -1)
  encode=$(echo "$output"  | grep "ffmpeg re-encode"         | grep -oE '[0-9]+\.[0-9]+' | head -1)
  sign=$(echo "$output"    | grep "C2PA signing"             | grep -oE '[0-9]+\.[0-9]+' | head -1)
  wall=$(echo "$output"    | grep "^│ Total"                 | grep -oE '[0-9]+\.[0-9]+' | head -1)

  total_decode+=("${decode:-0}")
  total_tile+=("${tile:-0}")
  total_hash+=("${hash:-0}")
  total_prove+=("${prove:-0}")
  total_encode+=("${encode:-0}")
  total_sign+=("${sign:-0}")
  total_wall+=("${wall:-0}")
  echo ""
done

# Compute averages with python3 (always available on macOS/Linux)
python3 - <<PYEOF
def avg(lst):
    vals = [float(x) for x in lst if x]
    return sum(vals) / len(vals) if vals else 0.0

decode  = [$(IFS=,; echo "${total_decode[*]}" | tr ' ' ',')]
tile    = [$(IFS=,; echo "${total_tile[*]}"   | tr ' ' ',')]
hash_   = [$(IFS=,; echo "${total_hash[*]}"   | tr ' ' ',')]
prove   = [$(IFS=,; echo "${total_prove[*]}"  | tr ' ' ',')]
encode  = [$(IFS=,; echo "${total_encode[*]}" | tr ' ' ',')]
sign    = [$(IFS=,; echo "${total_sign[*]}"   | tr ' ' ',')]
wall    = [$(IFS=,; echo "${total_wall[*]}"   | tr ' ' ',')]

runs = len(wall)
print(f"=== averages over {runs} run(s) ===")
print(f"┌─────────────────────────────────────────┬──────────┐")
print(f"│ Phase                                   │      Avg │")
print(f"├─────────────────────────────────────────┼──────────┤")
print(f"│ ffmpeg decode                           │ {avg(decode):>6.2f}s │")
print(f"│ macroblock tiling                       │ {avg(tile):>6.2f}s │")
print(f"│ edit + hashing (h1, h2)                 │ {avg(hash_):>6.2f}s │")
print(f"│ ZK proving (Nova IVC + Groth16)         │ {avg(prove):>6.2f}s │")
print(f"│ ffmpeg re-encode                        │ {avg(encode):>6.2f}s │")
print(f"│ C2PA signing                            │ {avg(sign):>6.2f}s │")
print(f"├─────────────────────────────────────────┼──────────┤")
print(f"│ Total                                   │ {avg(wall):>6.2f}s │")
print(f"└─────────────────────────────────────────┴──────────┘")
PYEOF
