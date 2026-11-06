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
