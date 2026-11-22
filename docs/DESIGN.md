# Design

Why verdict is built the way it is. Each entry records a decision, the
alternatives considered, and the evidence. The contract itself is in the
[README](../README.md#the-decide-contract-v1).

## 1. The /decide contract

**Context:** The gateway has to work with a fake worker now and a trained model later, and the two need to be benchmarked against each other.

**Decision (fixed contract):** The `/decide` contract is fixed, so worker implementations can be swapped without touching the gateway, and each side can be built and tested independently of the other. Because every worker accepts the same requests and returns the same response shape, benchmarks are directly comparable: any difference in latency comes from the worker, not the interface.

**Decision (strict validation):** Any request that breaks a contract rule is rejected with a 422 and a clear error, never corrected or guessed at. A bug in the caller's code then shows up straight away instead of producing plausible but wrong results. For example, a request with `min >= max` is rejected rather than corrected.

**Alternatives (strict validation):**
- Correcting bad input, e.g. swapping `min` and `max`: hides bugs in the caller's code, doesn't handle `min == max`, and leaves the labels describing the wrong values.

**Decision (ties):** When options share the highest probability, `answer` is the first in declared order: the first listed for `choice`, `yes` for `yes_no`, the lowest value for `score`. This creates a positional bias, which is acceptable because exact ties are very rare with a real model.

**Alternatives (ties):**
- Alphabetical: moves the bias from list position to options starting with "a".
- Random: not deterministic, so tests become flaky and cached answers could differ from the worker's.
- Random, seeded from a hash of the input: deterministic, but more complex to implement and harder to document.
- Reporting the tie: honest, but every caller would have to handle a tie case, so `answer` would no longer be a simple branch.

**Decision (`min` and `max`):** `min` and `max` are `i8`, so they must be between −128 and 127; serde rejects anything outside that range with a 422 before validation runs. The scale's width is computed in `i16`, because two `i8` values can be up to 255 apart, which would overflow `i8`.

**Alternatives (`min` and `max`):**
- A wider type such as `i32`: allows larger values, but only moves the overflow problem to extreme inputs; the subtraction would still need care.
- `checked_sub` in `i8`: also safe, but turns overflow into a `None` that has to be handled as a separate error, which is more code than widening.

**Consequences:**
- The trained model in Phase 5 can replace the fake worker without any gateway changes. The cost is that any change to the contract means updating every worker and client together.
- Callers get precise errors, but the API is unforgiving: a client with a small bug gets a 422 instead of an answer.
- Callers can't use scales with values outside −128 to 127. No realistic rating scale needs them, and the limit is documented in the contract's Clarifications.

## 2. Fake worker model

**Context:** The real model doesn't exist yet, but the gateway needs something behind it to build and benchmark against. The fake worker implements the full `/decide` contract and simulates a model's behaviour.

**Decision (latency model):** Each request sleeps for a simulated forward-pass cost:

latency = base + (input bytes × per-byte cost) + (options × per-option cost)

- Base: 20 ms, the fixed cost of any forward pass, whatever the input.
- Per byte: 10 µs, because reading a longer input takes longer.
- Per option: 2 ms, the cost of scoring each declared option.

These values were chosen to give realistic-looking latencies in the tens of milliseconds. They are not measured from a real model.

**Alternatives (latency):**
- Fixed latency for every request: `choice_short` and `choice_long` would cost exactly the same, so benchmarks couldn't show how payload size affects throughput and latency, which the M6 batching results depend on.

**Decision (probabilities):** Each option's weight comes from hashing the input together with the option, and weights are normalised so the probabilities sum to 1. The same request always gets the same probabilities; changing one character of the input reshuffles them.

**Alternatives (probabilities):**
- Random probabilities: the worker would give a different answer every time it is asked. Once the M2 cache exists, a cache hit and a cache miss would then return different answers to the same request, so the cache would change the system's behaviour rather than only its speed. Results would also not be reproducible across test and benchmark runs.

**Decision (fast rejects):** Invalid requests are rejected before the simulated forward pass, without sleeping, because a real model server would reject them before running inference.

**Consequences:**
- The probabilities are nearly flat: the first `choice_short` test with 3 options gave 0.31, 0.34 and 0.35. A real model is usually more decisive.
- `DefaultHasher` is stable between runs of the same build, but not guaranteed across Rust versions, so the exact probabilities may change after a toolchain upgrade.
- In a real transformer, attention cost grows roughly with the square of the input length (in tokens), so a linear per-byte cost understates the latency of long inputs.
- Benchmarks should mostly use valid requests: invalid ones measure validation code, not the simulated model.

**Evidence:** For `choice_short.json` (42 bytes, 3 options) the formula predicts 20 + (42 × 0.01) + (3 × 2) = 26.42 ms. Two identical requests measured 27.80 ms and 28.11 ms, about 1.4–1.7 ms above the prediction. The gap comes from Tokio's timer, which works at roughly 1 ms resolution so sleeps round up, plus the time the handler spends hashing, building the response and serializing it. The 0.3 ms difference between the two identical requests is run-to-run jitter, which is why benchmarks report p50 and p99 over many requests rather than single measurements.

## 3. Gateway and M1 baseline

**Context:** M1 puts a gateway in front of one worker and measures it against direct access to the worker. The result is the baseline that later milestones (cache, load balancing, batching) are compared against.

**Decision (raw-byte forwarding):** The gateway forwards the request body unchanged, together with the client's `content-type` header, and returns the worker's status code, `content-type` and body unchanged. It does not parse requests; the worker enforces the contract. All 46 contract tests pass through the gateway exactly as they do against the worker directly, including 400, 415 and 422 cases, so the gateway is transparent to clients.

**Alternatives (raw-byte forwarding):**
- Parse into `DecideRequest` and re-serialize: lets the gateway reject bad requests before the network hop, and is needed later for cache keys based on meaning and for batching. Rejected for M1 because a baseline should measure the minimum forwarding cost, and it would duplicate the contract types in the gateway.
- **Revisit at M2:** with raw bytes as cache keys, two requests that differ only in whitespace or key order get different keys, which may lower the hit rate. The M2 hit-rate numbers will show whether this matters.

**Decision (shared HTTP client):** One `reqwest::Client` is created at startup and shared through axum state, so connections to the worker are pooled and reused. A client per request would open a new TCP connection every time, and the baseline would measure connection setup rather than the gateway.

**Decision (timeouts and errors):** Connecting to the worker times out after 200 ms. The whole request (sending and reading the body) times out after 700 ms by default, configurable with `UPSTREAM_TIMEOUT_MS`. A timeout returns 504; any other failure to reach the worker or read its reply returns 502. Both use the contract's JSON error body.

The 700 ms default is about 4× the fake worker's worst legitimate latency: 20 ms + (8,192 bytes × 10 µs) + (32 options × 2 ms) ≈ 166 ms. The headroom covers queueing under load.

Tests: a worker that accepts the connection and never answers (`nc -l`) returns 504 once the timeout fires; a stopped worker returns 502; with `UPSTREAM_TIMEOUT_MS=10`, every valid request returns 504 because the worker's shortest possible latency is about 24 ms. That last test shows the cost of a timeout set too short: a healthy worker, with every request that does real work failing.

**Alternatives (timeouts):**
- No timeout: a stuck worker would hold a gateway task, a connection and a waiting client indefinitely, gradually draining capacity.
- A shorter timeout: the highest measured p99 was 53 ms, so 200–300 ms would fit the "a few times p99" rule of thumb. Not chosen because the benchmarks did not include the worst-case payload (8 KB, 32 options, about 166 ms). To be revisited when the real model replaces the fake worker.

**Evidence:** Release builds, all three processes (load generator, gateway, worker) on one 8-core Apple Silicon laptop, `oha` closed-loop for 15 s per run after a 3 s warm-up. Results in `bench/results/`; the runs below are at commit `ea02fbd`.

`choice_short`, second run:

| Clients | Worker req/s | Gateway req/s | Worker p50 | Gateway p50 | Gateway overhead (p50) |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 34 | 34 | 28.64 ms | 29.09 ms | 0.45 ms |
| 8 | 276 | 270 | 28.79 ms | 29.51 ms | 0.72 ms |
| 32 | 1,089 | 1,049 | 29.37 ms | 30.54 ms | 1.17 ms |
| 128 | 4,282 | 4,219 | 29.67 ms | 30.13 ms | 0.46 ms |

- **Gateway cost:** 0.3–1.4 ms p50 across both runs and all three payloads, and at most 4.5% of throughput.
- **Little's Law holds:** throughput ≈ clients ÷ latency at every level (e.g. 128 ÷ 29.67 ms ≈ 4,318 req/s predicted, 4,282 measured). Throughput was still scaling linearly at 128 clients, so the system's limit was not reached.
- **The tail moves first:** from 1 to 128 clients, worker p50 rose by about 1 ms but p99 by about 4 ms (30.3 → 34.3 ms in the first run).
- **The latency model holds:** every payload measured about 2.3 ms above the formula in Entry 2 (timer resolution and handler overhead), e.g. `choice_long` predicted 42.4 ms, measured 44.8 ms.
- **Open-loop at 500 req/s:** gateway overhead 0.31 ms p50 and 0.35 ms p99, the cleanest overhead measurement. p99 was no worse than closed-loop, as expected at about 12% of capacity: with no queueing there are no stalls for coordinated omission to hide. A run near capacity would be needed to see a difference.

**Open question:** gateway overhead peaks at 32 clients (1.39 ms and 1.17 ms in two runs) and is lower at both 8 and 128. It reproduced, so it is not random noise, but the cause is unknown. One hypothesis: at moderate load the gateway's threads repeatedly go idle and must be woken, while at 128 clients there is always work queued. Profiling would be needed to confirm it.

**Consequences:**
- The baseline for later milestones is about 30 ms p50 and 4,200 req/s through the gateway at 128 clients for `choice_short`.
- The load generator shares the laptop's CPUs with the servers, so these numbers are a lower bound on what the gateway and worker could do alone.
- `ErrorBody` is duplicated in the worker and the gateway. Shared types could move to a common crate when there is more to share.