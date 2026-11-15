# Contributing

Thanks for helping! Nex Speed is small on purpose: every feature must be cheap when idle.

- Build and test: `cargo test --workspace`, run: `cargo run -p nex-app --release`.
- All OS-specific code lives in `crates/nex-platform`. Pure logic goes in `nex-core` with unit tests
  (no OS or UI dependencies, so it runs on any machine).
- Every control in the UI must do something. If an OS cannot support a feature, hide it.
- No telemetry, accounts or background servers.
- Keep new dependencies minimal (`default-features = false` where possible) and say why in the PR.
- By contributing you agree your code is released under GPL-3.0-or-later.
