#!/usr/bin/env bash
# Checks that a server speaks the /decide contract (README, v1 + Clarifications).
# Point it at a worker or the gateway; both must give the same answers.
#
#   bench/contract_test.sh                         # gateway on :8080
#   bench/contract_test.sh http://127.0.0.1:9001   # worker directly
#
# Accepted requests: bench/payloads/*.json and bench/payloads/contract/ok/*.json
#   must return 200 and a response that obeys every response rule. The expected
#   options are worked out from each payload, so new cases need no script edits.
# Rejected requests: bench/payloads/contract/reject/*.json must return 422.
#   400, 415 and error precedence are checked inline below. Every error must be
#   served as application/json with a non-empty string `error`.
#
# Out of scope: 502 and 504.
set -euo pipefail

URL="${1:-${GATEWAY_URL:-http://127.0.0.1:8080}}"
DIR="$(cd "$(dirname "$0")" && pwd)"
EXPECTED_TOL=0.001 # contract: `expected` matches the returned probabilities within ±0.001
pass=0; fail=0

# post <content-type> <curl data arg>; sets $status, $ctype and $body
post() {
  local out
  out="$(curl -sS -m 5 -w '\n%{content_type}\n%{http_code}' -H "content-type: $1" --data-binary "$2" "$URL/decide")" || return 1
  status="${out##*$'\n'}"; out="${out%$'\n'*}"
  ctype="${out##*$'\n'}";  body="${out%$'\n'*}"
}

report() { # name errors
  if [[ -n "$2" ]]; then
    echo "FAIL $1:"; sed 's/^/  - /' <<<"$2"; fail=$((fail+1))
  else
    echo "ok   $1"; pass=$((pass+1))
  fi
}

expect_ok() { # name payload-file [content-type]
  local name="$1" payload="$2" errors
  post "${3:-application/json}" "@$payload" || { report "$name" "request error"; return; }
  if [[ "$status" != "200" ]]; then report "$name" "HTTP $status, want 200: ${body:0:200}"; return; fi

  # Options in declared order, derived from the request.
  local opts
  opts="$(jq -c '.question as $q |
    if   $q.type == "choice" then $q.options
    elif $q.type == "yes_no" then ["yes", "no"]
    else [range($q.min; $q.max + 1) | tostring] end' "$payload")"

  errors="$(jq -r --arg type "$(jq -r .question.type "$payload")" --argjson opts "$opts" --argjson tol "$EXPECTED_TOL" '
    [ (if .type != $type then "type is \(.type|tojson), want \($type)" else empty end),
      (if (.probabilities|type) != "object" then "probabilities is not an object"
       else
         (if (.probabilities|keys|sort) != ($opts|sort) then "probability keys \(.probabilities|keys) != options \($opts)" else empty end),
         (if any(.probabilities[]; type != "number" or . < 0 or . > 1) then "a probability is outside [0,1]" else empty end),
         (if ((.probabilities|add) - 1 | fabs) > 0.001 then "probabilities sum to \(.probabilities|add)" else empty end),
         ( . as $r | ([$r.probabilities[]]|max) as $m
           | ([$opts[] | select($r.probabilities[.] == $m)] | first) as $want
           | if $r.answer != $want then "answer is \($r.answer|tojson), want \($want|tojson) (most probable, ties to first in declared order)" else empty end),
         (if $type == "score" then
            ( . as $r | ([$r.probabilities | to_entries[] | (.key|tonumber) * .value] | add) as $mean
              | if ($r.expected|type) != "number" then "expected missing or not a number"
                elif (($r.expected - $mean) | fabs) > $tol then "expected is \($r.expected), probability-weighted mean is \($mean)"
                else empty end)
          elif has("expected") then "expected must be absent for \($type), got \(.expected|tojson)"
          else empty end)
       end),
      (if (.model|type) != "string" or .model == "" then "model missing or empty" else empty end),
      (if (.latency_ms|type) != "number" or .latency_ms < 0 then "latency_ms missing or negative" else empty end)
    ] | .[]' <<<"$body" 2>&1)" || errors="response is not valid JSON: ${body:0:200}"
  report "$name" "$errors"
}

expect_error() { # name want-status content-type curl-data-arg
  local name="$1" want="$2" errors=""
  post "$3" "$4" || { report "$name" "request error"; return; }
  if [[ "$status" != "$want" ]]; then
    errors="HTTP $status, want $want: ${body:0:200}"
  else
    # Media type is case-insensitive and may carry parameters such as charset.
    local media="${ctype%%;*}"; media="$(tr '[:upper:]' '[:lower:]' <<<"${media// /}")"
    if [[ "$media" != "application/json" ]]; then
      errors="served as \"${ctype:-<no content-type>}\", want application/json"
    fi
    if ! jq -e '(.error|type) == "string" and .error != ""' >/dev/null 2>&1 <<<"$body"; then
      errors="${errors:+$errors$'\n'}body has no non-empty string \"error\": ${body:0:200}"
    fi
  fi
  report "$name" "$errors"
}

echo "contract test against $URL"
curl -s -m 2 -o /dev/null "$URL/decide" || { echo "$URL is not answering; start it first"; exit 1; }

echo "-- accepted"
for f in "$DIR"/payloads/*.json "$DIR"/payloads/contract/ok/*.json; do
  expect_ok "$(basename "$f" .json)" "$f"
done
expect_ok content_type_with_charset "$DIR/payloads/yes_no.json" 'application/json; charset=utf-8'

echo "-- rejected"
for f in "$DIR"/payloads/contract/reject/*.json; do
  expect_error "$(basename "$f" .json)" 422 application/json "@$f"
done
expect_error invalid_json             400 application/json '{"input": "x", "question": {"type": "yes_no"'
expect_error content_type_text_plain  415 text/plain       "@$DIR/payloads/yes_no.json"
# Breaks all three levels at once: wrong media type, malformed JSON (truncated),
# and content that breaks 422 rules (empty input, min >= max). 415 must win.
expect_error precedence_415_over_400_and_422 415 text/plain '{"input": "", "question": {"type": "score", "min": 5, "max": 1'

echo "$pass passed, $fail failed"
[[ $fail -eq 0 ]]
