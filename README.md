# oxidtr

Alloy formal specification models to deterministic code generation, test scaffolding, and structural consistency verification.

> **oxidtr** is short for *oxidator* (oxidizing agent). [Alloy](https://alloytools.org/) means "alloy" (metal), [Rust](https://www.rust-lang.org/) means "rust" (iron oxide) — oxidtr is the catalyst that turns one into the other.

## What it does

oxidtr takes an [Alloy](https://alloytools.org/) model (`.als`) as the single source of truth and deterministically generates:

- **Type definitions** — structs, enums, interfaces, records, sealed classes with multiplicity-aware types
- **Operation stubs** — function signatures from predicates, with empty bodies for human/AI implementation
- **Invariant functions** — fact constraints translated to executable boolean functions
- **Property tests** — assert declarations translated to test cases
- **Cross-tests** — fact × predicate preservation test scaffolding
- **Transitive closure traversal** — generated BFS/chain-walk functions for `^field` expressions
- **Fixtures** — factory functions generating valid default instances
- **Doc comments** — constraint names as rustdoc / JSDoc / KDoc / Javadoc annotations
- **JSON Schema** — structural schema from model types and constraints
- **Bean Validation** — `@NotNull` annotations on Java record fields

It also provides:
- **Structural consistency checking** between Alloy models and implementations (Rust / TypeScript / Kotlin / Java)
- **Model mining** — extract Alloy model drafts from existing source code or JSON Schema

## Design principles

- **All commands are deterministic.** No AI dependency. No probabilistic elements in generation.
- **The model is the single source of truth.** Types, tests, and consistency checks are all derived from the Alloy model.
- **Guarantee budget is constant.** Stronger type systems reduce test generation; weaker ones increase it.
- **Minimal dependencies.** No serde, no tree-sitter. Parsers are hand-written for oxidtr's own formats.

## Architecture

```
Alloy source (.als)
  → Parser → Alloy AST
  → Lowering → oxidtr IR (StructureNode, ConstraintNode, OperationNode, PropertyNode)
  → Target Backend → Generated code (Rust / TypeScript / Kotlin / Java)
  → Constraint Analyzer → JSON Schema, doc comments, fixtures, annotations
```

## Supported targets

| Target | Flag | Types | Tests | Fixtures | Docs | Extra |
|---|---|---|---|---|---|---|
| Rust | `--target rust` | struct, enum | proptest-style | `impl Default` builder | rustdoc | newtype-ready |
| TypeScript | `--target ts` | interface, union | vitest | factory functions | JSDoc | — |
| Kotlin | `--target kt` | data class, sealed/enum class | JUnit 5 | companion factory | KDoc | — |
| Java | `--target java` | record, sealed interface, enum | JUnit 5 | static factory | Javadoc | `@NotNull` |

## Commands

### `oxidtr generate`

Generate types, tests, operation stubs, fixtures, and doc comments from an Alloy model.

```
oxidtr generate model.als --target rust --output generated/
oxidtr generate model.als --target ts --output generated-ts/
oxidtr generate model.als --target kt --output generated-kt/
oxidtr generate model.als --target java --output generated-java/
```

Detects 7 structural warnings during generation:

| Warning | Condition |
|---|---|
| `UNCONSTRAINED_SELF_REF` | Self-referential field with no constraint |
| `UNCONSTRAINED_CARDINALITY` | `set` field with no cardinality fact |
| `MISSING_INVERSE` | Bidirectional fields without inverse fact |
| `UNREFERENCED_SIG` | Sig referenced by no other sig, constraint, or predicate |
| `UNCONSTRAINED_TRANSITIVITY` | `^field` used but no direct fact on field |
| `UNHANDLED_RESPONSE_PATTERN` | Abstract sig variant with no predicate |
| `MISSING_ERROR_PROPAGATION` | Error variant with no predicate |

### `oxidtr check`

Verify structural consistency between an Alloy model and implementation. Auto-detects language by file presence.

```
oxidtr check --model model.als --impl src/
```

Detects: `MISSING_STRUCT`, `EXTRA_STRUCT`, `MISSING_FIELD`, `EXTRA_FIELD`, `MULTIPLICITY_MISMATCH`, `MISSING_FN`, `EXTRA_FN`. Non-zero exit on any diff — use as a CI gate.

### `oxidtr mine`

Extract Alloy model drafts from existing source code.

```
oxidtr mine --source src/models.rs --lang rust
oxidtr mine --source src/models.ts --lang ts
oxidtr mine --source src/Models.kt --lang kt
oxidtr mine --source src/Models.java --lang java
oxidtr mine --source schemas.json --lang schema
```

Produces Alloy `.als` text with sig/field/multiplicity extraction and fact candidates with confidence levels (High / Medium / Low).

## Self-hosting

oxidtr's own domain is modeled in `models/oxidtr.als`. The full round-trip is verified for all targets:

```
oxidtr.als → generate (Rust/TS/Kotlin/Java) → check → 0 diffs
oxidtr.als → generate → mine → structural match with original
oxidtr.als → generate → schemas.json → mine --lang schema → structural match
```

## Development

```bash
cargo test              # 206 tests
cargo run -- generate models/oxidtr.als --target rust --output generated
cargo run -- check --model models/oxidtr.als --impl generated
cargo run -- mine --source generated/models.rs --lang rust
```

## Roadmap

### Completed

| Phase | Description |
|---|---|
| 1 | Parser + IR + Rust backend (type generation) |
| 2 | Expression translation + test generation + TC type inference + self-hosting |
| 3 | check command + all 7 generate warnings |
| 4 | TypeScript backend + mine (Rust/TS) + round-trip verification |
| 5 | Kotlin/Java backends (shared JVM layer) + mine extractors |
| 6 | Enrichment: fixtures, doc comments, JSON Schema, Bean Validation, schema mine |

### Planned

| Phase | Description | Target platforms |
|---|---|---|
| 7 | **Swift backend** | iOS / macOS / visionOS |
| 8 | **Go backend** | Cloud-native / CLI / microservices |
| 9 | **C# backend** | .NET / Unity / Blazor / Azure |
| 10 | **Lean backend** (polarstar) | High-assurance domains |
| — | explore | Alloy instance anomaly detection |
| — | cover | Coverage × fact orthogonal test generation |

### Language guarantee spectrum

```
More tests ← ─────────────────────────────── → More proofs
  TypeScript   Go   Java   C#   Kotlin   Swift   Rust   Lean
     △          △     ○      ○     ◎       ◎      ◎      ◎ ← type safety
     ✕          △     △      △     △       ○      ○      ◎ ← effect control
     △          △     △      △     △       ○      ○      ◎ ← invariant encoding
     ✕          ✕     ✕      ✕     ✕       ✕      ✕      ◎ ← theorem proving
```

The Lean backend (polarstar) is the endpoint of oxidtr's roadmap — where Alloy facts become Lean theorems, and verification shifts from runtime testing to static proof.

## License

MIT
