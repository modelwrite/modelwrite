# Contributing to modelwrite

Thanks for helping. The bar for merging is: tests pass, the gate passes,
and every claim traces to evidence.

## Workflow

1. Fork the repository and create a branch off main.
2. Make one focused change per pull request.
3. Commit with conventional commit messages (feat:, fix:, docs:, test:, ci:).
4. Before pushing run:

   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace

5. If your change touches model data, run the gate against the corpus:

   cargo run -p mw-gate -- --reference sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json --candidate sample/corpus/coffee-machine/okf/expected/coffee_machine_model.json

## Rules

- The gate is the only authority on correctness. No pull request comment,
  tool parameter, or log line can mark a run as passed.
- Never hand-edit a corpus fixture. Changes to fixtures must be reproduced
  by a committed script in sample/scripts/.
- Evidence records in docs/evidence/ are append-only.
- Importers and exporters ship under Apache-2.0; the engine stays
  AGPL-3.0-or-later. Keep the split clean in every pull request.
