#!/usr/bin/env bash
# Checks that a server speaks the /decide contract (README, draft v0).
# Point it at a worker or the gateway; both must give the same shape of answer.
#
#   bench/contract_test.sh                         # gateway on :8080
#   bench/contract_test.sh http://127.0.0.1:9001   # worker directly
#
# Covers choice and yes_no. Score is skipped: the contract does not yet say
# which request fields carry the scale.
set -euo pipefail

URL="${1:-${GATEWAY_URL:-http://127.0.0.1:8080}}"
DIR="$(cd "$(dirname "$0")" && pwd)"
pass=0; fail=0

check() {
  local name="$1" payload="$2" expected_options="$3"
  local out status body
  out="$(curl -sS -m 5 -w '\n%{http_code}' -H 'content-type: application/json' \
         --data-binary @"$payload" "$URL/decide")" || { echo "FAIL $name: request error"; fail=$((fail+1)); return; }
  status="${out##*$'\n'}"; body="${out%$'\n'*}"

  if [[ "$status" != "200" ]]; then
    echo "FAIL $name: HTTP $status: $body"; fail=$((fail+1)); return
  fi

  local errors
  errors="$(jq -r --arg type "$(jq -r .question.type "$payload")" --argjson opts "$expected_options" '
    [ (if .type != $type then "type is \(.type|tojson), want \($type)" else empty end),
      (if (.probabilities|type) != "object" then "probabilities is not an object"
       else
         (if (.probabilities|keys|sort) != ($opts|sort) then "probability keys \(.probabilities|keys) != options \($opts)" else empty end),
         (if any(.probabilities[]; type != "number" or . < 0 or . > 1) then "a probability is outside [0,1]" else empty end),
         (if ((.probabilities|add) - 1 | fabs) > 0.001 then "probabilities sum to \(.probabilities|add)" else empty end),
         (if .probabilities[.answer] != ([.probabilities[]]|max) then "answer \(.answer|tojson) is not the most probable option" else empty end)
       end),
      (if (.model|type) != "string" or .model == "" then "model missing" else empty end),
      (if (.latency_ms|type) != "number" or .latency_ms < 0 then "latency_ms missing or negative" else empty end)
    ] | .[]' <<<"$body" 2>&1)" || errors="response is not valid JSON: $body"

  if [[ -n "$errors" ]]; then
    echo "FAIL $name:"; sed 's/^/  - /' <<<"$errors"; fail=$((fail+1))
  else
    echo "ok   $name"; pass=$((pass+1))
  fi
}

echo "contract test against $URL"
check choice_short "$DIR/payloads/choice_short.json" '["billing","technical","account"]'
check choice_long  "$DIR/payloads/choice_long.json"  '["billing","technical","account","sales","security","feedback","shipping","other"]'
check yes_no       "$DIR/payloads/yes_no.json"       '["yes","no"]'

echo "$pass passed, $fail failed"
[[ $fail -eq 0 ]]
