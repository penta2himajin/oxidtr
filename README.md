# oxidtr

Alloy formal specification models to deterministic code generation, test scaffolding, and structural consistency verification.

> **oxidtr** is short for *oxidator* (oxidizing agent). [Alloy](https://alloytools.org/) means "alloy" (metal), [Rust](https://www.rust-lang.org/) means "rust" (iron oxide) — oxidtr is the catalyst that turns one into the other.

## What it does

oxidtr takes an [Alloy](https://alloytools.org/) model (`.als`) as the single source of truth and deterministically generates:

- **Type definitions** — structs, enums with variant fields, multiplicity-aware types (`Option<T>`, `Vec<T>`, `Box<T>`)
- **Operation stubs** — function signatures from predicates, with `todo!()` bodies for human/AI implementation
- **Invariant functions** — fact constraints translated to executable Rust boolean functions
- **Property tests** — assert declarations translated to test cases
- **Cross-tests** — fact × predicate preservation test scaffolding
- **Transitive closure traversal** — generated BFS/chain-walk functions for `^field` expressions

It also provides structural consistency checking between Alloy models and existing Rust implementations.

## Design principles

- **All commands are deterministic.** No AI dependency. No probabilistic elements in generation.
- **The model is the single source of truth.** Types, tests, and consistency checks are all derived from the Alloy model.
- **Guarantee budget is constant.** Stronger type systems reduce test generation; weaker ones increase it.

## Architecture

```
Alloy source (.als)
  → Parser → Alloy AST
  → Lowering → oxidtr IR (StructureNode, ConstraintNode, OperationNode, PropertyNode)
  → Rust Backend → Generated Rust code
```

## Commands

### `oxidtr generate`

Generate types, tests, and operation stubs from an Alloy model.

```
oxidtr generate model.als --target rust --output generated/
```

Detects 7 structural warnings during generation:

| Warning | Condition |
|---|---|
| `UNCONSTRAINED_SELF_REF` | Self-referential field with no constraint |
| `UNCONSTRAINED_CARDINALITY` | `set` field with no cardinality fact |
| `MISSING_INVERSE` | Bidirectional fields without inverse fact |
| `UNREFERENCED_SIG` | Sig referenced by no other sig |
| `UNCONSTRAINED_TRANSITIVITY` | `^field` used but no direct fact on field |
| `UNHANDLED_RESPONSE_PATTERN` | Abstract sig variant with no predicate |
| `MISSING_ERROR_PROPAGATION` | Error variant with no predicate |

### `oxidtr check`

Verify structural consistency between an Alloy model and Rust implementation.

```
oxidtr check --model model.als --impl src/
```

Detects: `MISSING_STRUCT`, `EXTRA_STRUCT`, `MISSING_FIELD`, `EXTRA_FIELD`, `MULTIPLICITY_MISMATCH`, `MISSING_FN`, `EXTRA_FN`. Non-zero exit on any diff — use as a CI gate.

## Self-hosting

oxidtr's own domain is modeled in `models/oxidtr.als`. Running `oxidtr generate` on this model and then `oxidtr check` against the generated output produces zero diffs.

## Development

```bash
cargo test          # 112 tests
cargo run -- generate models/oxidtr.als --output generated
cargo run -- check --model models/oxidtr.als --impl generated
```

## Status

| Phase | Description | Status |
|---|---|---|
| 1 | Parser + IR + Rust backend (type generation) | Done |
| 2 | Expression translation + test generation + TC type inference + self-hosting | Done |
| 3 | check command + all 7 generate warnings | Done |
| 4 | TypeScript backend | Planned |
| 5 | Kotlin backend | Planned |
| 6 | PureScript / Haskell backend | Planned |

## License

MIT
