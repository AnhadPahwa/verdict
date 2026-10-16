#!/usr/bin/env bash
# M1 baseline: throughput and p50/p99 for the gateway forwarding to one fake worker.
#
# Each payload runs at each concurrency level twice: once straight at the worker
# and once through the gateway. The difference is the gateway's own cost.
#
# Start both release binaries first, then:
#   bench/m1.sh
#
# Settings (environment variables):
#   GATEWAY_URL   default http://127.0.0.1:8080
#   WORKER_URL    default http://127.0.0.1:9001
#   DURATION      per run, default 15s
#   CONCURRENCY   space-separated, default "1 8 32 128"
#   PAYLOADS      space-separated names from bench/payloads, default all
#   RATE          if set (requests/sec), also runs open-loop at that rate with
#                 coordinated-omission correction; closed-loop p99 hides queueing
#
# Raw oha JSON and a summary.md go to bench/results/m1-<timestamp>/.
set -euo pipefail

command -v oha >/dev/null || { echo "oha not found: brew install oha (or cargo install oha)"; exit 1; }
command -v jq  >/dev/null || { echo "jq not found"; exit 1; }

DIR="$(cd "$(dirname "$0")" && pwd)"
GATEWAY_URL="${GATEWAY_URL:-http://127.0.0.1:8080}"
WORKER_URL="${WORKER_URL:-http://127.0.0.1:9001}"
DURATION="${DURATION:-15s}"
CONCURRENCY="${CONCURRENCY:-1 8 32 128}"
PAYLOADS="${PAYLOADS:-$(cd "$DIR/payloads" && ls *.json | sed 's/\.json$//' | tr '\n' ' ')}"
RATE="${RATE:-}"

OUT="$DIR/results/m1-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT"

for target in "$WORKER_URL" "$GATEWAY_URL"; do
  curl -sf -m 2 -o /dev/null -H 'content-type: application/json' \
    --data-binary @"$DIR/payloads/yes_no.json" "$target/decide" \
    || { echo "$target/decide is not answering; start it first"; exit 1; }
done

{
  echo "git:      $(git -C "$DIR" rev-parse --short HEAD)$(git -C "$DIR" diff --quiet || echo '-dirty')"
  echo "date:     $(date -u +%FT%TZ)"
  echo "host:     $(uname -sm), $(sysctl -n hw.ncpu 2>/dev/null || nproc) cpus"
  echo "duration: $DURATION   concurrency: $CONCURRENCY"
  echo "gateway:  $GATEWAY_URL   worker: $WORKER_URL"
} > "$OUT/meta.txt"
cat "$OUT/meta.txt"

run() { # name url payload extra-args...
  local name="$1" url="$2" payload="$3"; shift 3
  echo "  $name"
  echo "$name" >> "$OUT/runs.txt"
  oha --no-tui --output-format json -z "$DURATION" -m POST -T application/json \
      -D "$DIR/payloads/$payload.json" "$@" "$url/decide" > "$OUT/$name.json"
}

# Warm up connections, allocators and caches so the first measured run isn't penalised.
oha --no-tui --output-format quiet -z 3s -c 8 -m POST -T application/json \
    -D "$DIR/payloads/yes_no.json" "$GATEWAY_URL/decide"

for payload in $PAYLOADS; do
  for c in $CONCURRENCY; do
    run "worker_${payload}_c$c"  "$WORKER_URL"  "$payload" -c "$c"
    run "gateway_${payload}_c$c" "$GATEWAY_URL" "$payload" -c "$c"
  done
  if [[ -n "$RATE" ]]; then
    run "worker_${payload}_q$RATE"  "$WORKER_URL"  "$payload" -c 256 -q "$RATE" --latency-correction
    run "gateway_${payload}_q$RATE" "$GATEWAY_URL" "$payload" -c 256 -q "$RATE" --latency-correction
  fi
done

# One row per run. Latencies in ms.
{
  echo "| run | req/s | p50 ms | p99 ms | success |"
  echo "| --- | ---: | ---: | ---: | ---: |"
  while read -r n; do
    jq -r --arg n "$n" '
      "| \($n) | \(.summary.requestsPerSec|floor) | \(.latencyPercentiles.p50*1000*100|round/100) | \(.latencyPercentiles.p99*1000*100|round/100) | \(.summary.successRate*100*10|round/10)% |"' "$OUT/$n.json"
  done < "$OUT/runs.txt"
} > "$OUT/summary.md"

echo; cat "$OUT/summary.md"
echo; echo "raw results: $OUT"
