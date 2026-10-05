# verdict
Rust gateway + inference workers behind a fixed /decide contract (see README).

## Rules
- I write core logic: forwarding, caching, rate limiting, batching, model code.
  Do not write these. Review them, point out bugs, suggest tests.
- You may write: config, Dockerfiles, CI, test and benchmark scripts.
- Never change the /decide contract without asking.