# oxidtr

Alloy formal specification models to deterministic code generation, test scaffolding, and structural consistency verification.

> **oxidtr** is short for *oxidator* (oxidizing agent). [Alloy](https://alloytools.org/) means "alloy" (metal), [Rust](https://www.rust-lang.org/) means "rust" (iron oxide) — oxidtr is the catalyst that turns one into the other.

## What it does

oxidtr takes an [Alloy](https://alloytools.org/) model (`.als`) as the single source of truth and deterministically generates:

- **Type definitions** — structs, enums, interfaces, records, sealed classes with multiplicity-aware types (`Set`/`Seq` distinction, `Map` for relation products, singleton for `one sig`)
- **Operation stubs** — function signatures from predicates and funs, with typed returns and `@pre`/`@post` doc comments
- **Invariant functions** — fact constraints translated to executable boolean functions with `@alloy:` comment preserving original Alloy syntax
- **Property tests** — assert declarations translated to non-vacuous test cases using fixture data
- **Cross-tests** — fact × predicate preservation test scaffolding with boundary values
- **Transitive closure traversal** — generated BFS/chain-walk functions for `^field` expressions
- **Fixtures** — factory functions with default, boundary, and violation instances from constraint analysis
- **Newtypes** — Rust `TryFrom` validated wrappers with concrete range checks from cardinality bounds
- **Doc comments** — constraint names, `@pre`/`@post` conditions, `@NotEmpty`/`@unique` annotations
- **JSON Schema** — structural schema with `minItems`/`maxItems`, `uniqueItems`, nullable, `additionalProperties` for maps, set operation descriptions
- **Bean Validation** — `@NotNull`, `@Size(min=N, max=M)`, `@NotEmpty` on Java/Kotlin fields
- **Serde opt-in** — `#[derive(Serialize, Deserialize)]` with `--features serde`

It also provides:
- **Structural consistency checking** between Alloy models and implementations (Rust / TypeScript / Kotlin / Java / Swift / Go), auto-detecting language
- **Model mining** — extract Alloy model drafts from existing source code, JSON Schema, or mixed multi-language directories with conflict detection
- **Lossless round-trip** — `@alloy:` comments preserve original expressions; reverse translation recovers Alloy from generated code

## Design principles

- **All commands are deterministic.** No AI dependency. No probabilistic elements in generation.
- **The model is the single source of truth.** Types, tests, and consistency checks are all derived from the Alloy model.
- **Guarantee budget is constant.** Stronger type systems reduce test generation; weaker ones increase it.
- **Minimal dependencies.** No serde, no tree-sitter. Parsers are hand-written for oxidtr's own formats.

## Alloy coverage

The parser handles the full Alloy structural grammar:

| Alloy construct | Support |
|---|---|
| `sig` / `abstract sig` / `one`/`some`/`lone sig` | Full |
| `extends` (inheritance) | Full |
| `one` / `lone` / `set` / `seq` field multiplicity | Full, with Set/Seq distinction |
| `->` relation product | Maps (BTreeMap, Map) |
| `fact` / `assert` / `pred` / `fun` | Full, with return types |
| `all` / `some` / `no` quantifiers | Full, multi-variable, `disj` |
| `and` / `or` / `implies` / `iff` / `not` | Full |
| `in` / `=` / `!=` / `<` / `>` / `<=` / `>=` | Full, with integer literals |
| `#` (cardinality) | Full, with numeric bound extraction |
| `^` (transitive closure) | Full |
| `+` / `&` / `-` (set operations) | Full |
| `var` field modifier (Alloy 6) | Full |
| `x'` prime operator (Alloy 6) | Full |
| `always` / `eventually` / `after` / `historically` / `once` / `before` (Alloy 6) | Full (invariant/transition validators) |
| `check` / `run` commands | Skipped (design-time only) |

## Supported targets

| Target | Flag | Types | Tests | Fixtures | Docs | Extra |
|---|---|---|---|---|---|---|
| Rust | `--target rust` | struct, BTreeSet, BTreeMap, enum | proptest-style + boundary | default + boundary + violation | rustdoc + @alloy | newtype TryFrom, serde opt-in |
| TypeScript | `--target ts` | interface, Set, Map, union | vitest + boundary | factory + boundary + violation | JSDoc + @alloy | — |
| Kotlin | `--target kt` | data class, object, Set, Map, sealed/enum | JUnit 5 + boundary | factory + boundary + violation | KDoc + @alloy | @Size, @NotEmpty |
| Java | `--target java` | record, Set, Map, sealed interface, enum | JUnit 5 + boundary | static factory + boundary + violation | Javadoc + @alloy | @NotNull, @Size, compact constructor |
| Swift | `--target swift` | struct, Set, Array, enum w/ associated values | XCTest + boundary | factory + boundary + violation | Swift doc comments | CaseIterable, Equatable |
| Go | `--target go` | struct, iota enum, interface sum type | testing + boundary | factory + boundary + violation | Go doc comments | *T for optional, []T for collections |

## Commands

### `oxidtr generate`

Generate types, tests, operation stubs, fixtures, and doc comments from an Alloy model.

```
oxidtr generate model.als --target rust --output generated/
oxidtr generate model.als --target ts --output generated-ts/
oxidtr generate model.als --target kt --output generated-kt/
oxidtr generate model.als --target java --output generated-java/
oxidtr generate model.als --target swift --output generated-swift/
oxidtr generate model.als --target go --output generated-go/
oxidtr generate model.als --target rust --output generated/ --features serde
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

Verify structural consistency between an Alloy model and implementation. Auto-detects language by file presence (`models.rs` / `models.ts` / `Models.kt` / `Models.java` / `Models.swift` / `models.go`).

```
oxidtr check --model model.als --impl src/
```

Detects: `MISSING_STRUCT`, `EXTRA_STRUCT`, `MISSING_FIELD`, `EXTRA_FIELD`, `MULTIPLICITY_MISMATCH`, `MISSING_FN`, `EXTRA_FN`. Non-zero exit on any diff — use as a CI gate.

### `oxidtr extract`

Extract Alloy model drafts from existing source code. Auto-detects language from file extension or directory contents.

```
oxidtr extract generated/                    # directory → auto-detect, multi-lang merge
oxidtr extract src/models.rs                 # single file → auto-detect from extension
oxidtr extract src/ --lang rust              # explicit language override
oxidtr extract src/ --conflict error         # fail on cross-language conflicts
```

Supports: `.rs` (Rust), `.ts` (TypeScript), `.kt` (Kotlin), `.java` (Java), `.swift` (Swift), `.go` (Go), `.json` (JSON Schema).

Multi-language directories are merged: same-name sigs are unified, missing fields are supplemented, and conflicts (multiplicity/target type mismatches) are reported.

Produces Alloy `.als` text with:
- Sig/field/multiplicity extraction from type definitions
- `@alloy:` comment recovery for lossless fact/assert/pred round-trip
- Reverse expression translation (language code → Alloy expressions)
- Fact candidates with confidence levels (High / Medium / Low)

## Self-hosting

oxidtr's own domain is modeled in `models/oxidtr.als`. The full round-trip is verified for all targets:

```
oxidtr.als → generate (Rust/TS/Kotlin/Java/Swift/Go) → check → 0 diffs
oxidtr.als → generate → extract → structural + expression match with original
oxidtr.als → generate (all languages) → extract (multi-lang merge) → unified Alloy model
```

## Development

```bash
cargo test              # 395 tests
cargo run -- generate models/oxidtr.als --target rust --output generated
cargo run -- check --model models/oxidtr.als --impl generated
cargo run -- extract generated/
```

## Roadmap

### Completed

| Phase | Description |
|---|---|
| 1 | Parser + IR + Rust backend (type generation) |
| 2 | Expression translation + test generation + TC type inference + self-hosting |
| 3 | check command + all 7 generate warnings |
| 4 | TypeScript backend + extract (Rust/TS) + round-trip verification |
| 5 | Kotlin/Java backends (shared JVM layer) + extract extractors |
| 6 | Enrichment: fixtures, doc comments, JSON Schema, Bean Validation, schema extract |
| 6+ | Full Alloy parser (integers, set ops, product, fun, multi-var quantifiers, disj) |
| 6+ | Complete conversion: Set/Seq distinction, singletons, concrete values, maps, boundaries, @alloy lossless round-trip |
| 6+ | Multi-language extract merge with conflict detection |
| 7 | Swift backend (struct/enum, XCTest, allSatisfy/contains, extract extractor) |
| 8 | Go backend (struct/iota/interface, testing, extract extractor) |
| 11 | Alloy 6 temporal parser (var, prime, always/eventually/after/historically/once/before) |
| 12-13 | Alloy 6 temporal code generation (prime expr, temporal validators, var check) |

### Planned

| Phase | Description | Target platforms |
|---|---|---|
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
