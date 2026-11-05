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

**Context:** [Why a fake worker exists at all before the model does.]

**Decision:** [Latency formula with your constants; hashed probabilities.]

**Alternatives:** [Fixed latency; random probabilities; and why not.]

**Consequences:** [Fast rejects; nearly flat probabilities; hasher stability.]

**Evidence:** [Predicted vs measured latency for choice_short.json, and why they differ.]