# oxidtr

**Write your data model's invariants once in [Alloy](https://alloytools.org/), get types, validators, and tests in 8 languages — deterministically, with no AI in the loop.**

Rust, TypeScript, Kotlin, Java, Swift, Go, C#, and Lean, all generated from a single `.als` spec that stays the source of truth. It also checks existing code against the model and mines Alloy back out of it, so the model and the implementation can't silently drift.

> **oxidtr** is short for *oxidator* (oxidizing agent). [Alloy](https://alloytools.org/) means "alloy" (metal), [Rust](https://www.rust-lang.org/) means "rust" (iron oxide) — oxidtr is the catalyst that turns one into the other.

## Example

A tiny model — a task graph that must stay acyclic:

```alloy
// task.als
sig Task {
  deps: set Task
}

fact Acyclic {
  no t: Task | t in t.^deps        // no task transitively depends on itself
}

assert NoSelfDependency {
  no t: Task | t in t.deps
}
```

```
oxidtr generate task.als --target rust --output generated/
```

Out comes a struct, a **validated newtype that enforces the invariant**, the transitive-closure helper the invariant needs, and **real tests** — all from those 10 lines:

```rust
// generated/task/models.rs
/// Invariant: Acyclic
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Task {
    pub deps: BTreeSet<Task>,
}

// generated/newtypes.rs — construction that can't produce a cyclic Task
impl TryFrom<Task> for ValidatedTask {
    type Error = &'static str;
    fn try_from(value: Task) -> Result<Self, Self::Error> {
        let tasks = vec![value.clone()];
        if !tasks.iter().any(|t| tc_deps(t).contains(t)) {
            Ok(ValidatedTask(value))
        } else {
            Err("Acyclic invariant violated")
        }
    }
}

// generated/tests.rs — the assert becomes an executable test
#[test]
fn no_self_dependency() {
    let tasks = vec![default_task()];
    assert!(!tasks.iter().any(|t| t.deps.contains(t)));
}
```

Point it at `--target ts` instead and the same `fact` becomes a runtime validator:

```typescript
// generated/models.ts
export interface Task { readonly deps: Set<Task>; }

// generated/validators.ts  — @covers: Acyclic
export function validateTask(t: Task): string[] {
  const errors: string[] = [];
  { const seen = new Set<unknown>(); let cur: unknown = t;
    while (cur != null) {
      if (seen.has(cur)) { errors.push("deps must not form a cycle"); break; }
      seen.add(cur); cur = (cur as Record<string, unknown>).deps;
    } }
  return errors;
}
```

Swap `--target` for any of the eight backends below to get the equivalent types, validators, and tests in that language. Change the model, re-run, and `oxidtr check` tells you exactly where any hand-written implementation has drifted.

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
| C# | `--target cs` | class, enum, abstract class hierarchy | xUnit + boundary | factory + boundary + violation | XML doc comments | T? for nullable, List\<T> for collections |
| Lean | `--target lean` | structure, inductive | theorem + sorry | — | — | fact → theorem, expr translator (∀/∃/∧/∨/→/↔) |

## Model layout

Per the Alloy 6 spec, **one file = one module**, and a `module X` header may only appear at the top of a file. The recommended layout splits a model into one file per module and wires them together with `open` — this is what the Alloy Analyzer expects:

```
models/
  oxidtr-split.als        # main: open oxidtr/{ast,ir,analysis,validated}, cross-module facts
  oxidtr/
    ast.als               # module oxidtr/ast       (leaf)
    ir.als                # module oxidtr/ir         (open oxidtr/ast)
    analysis.als          # module oxidtr/analysis
    validated.als         # module oxidtr/validated
```

`generate`, `check`, and `extract` all take just the main file (or its directory) and follow `open` directives to resolve the whole module graph. Module paths are root-relative (`open oxidtr/ast` resolves from the main file's directory), exactly as in Alloy.

```
oxidtr generate models/oxidtr-split.als --target rust --output generated/
oxidtr check    --model models/oxidtr-split.als --impl generated/
```

A **legacy single-file form** — multiple `module X` blocks stacked in one `.als` — is still read (with a `DEPRECATED` warning) for backward compatibility, but it violates the Alloy spec and won't load in the Alloy Analyzer. Write new models split.

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
oxidtr generate model.als --target cs --output generated-cs/
oxidtr generate model.als --target lean --output generated-lean/
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

Verify structural consistency between an Alloy model and implementation. Auto-detects language by file presence (`models.rs` / `models.ts` / `Models.kt` / `Models.java` / `Models.swift` / `models.go` / `Models.cs` / `Types.lean`).

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

Supports: `.rs` (Rust), `.ts` (TypeScript), `.kt` (Kotlin), `.java` (Java), `.swift` (Swift), `.go` (Go), `.cs` (C#), `.lean` (Lean), `.json` (JSON Schema).

Multi-language directories are merged: same-name sigs are unified, missing fields are supplemented, and conflicts (multiplicity/target type mismatches) are reported.

Produces Alloy `.als` text with:
- Sig/field/multiplicity extraction from type definitions
- `@alloy:` comment recovery for lossless fact/assert/pred round-trip
- Reverse expression translation (language code → Alloy expressions)
- Fact candidates with confidence levels (High / Medium / Low)

## Self-hosting

oxidtr's own domain is modeled as a split module set in `models/oxidtr/` (main file `models/oxidtr-split.als`). The full round-trip is verified for all targets:

```
oxidtr-split.als → generate (Rust/TS/Kotlin/Java/Swift/Go/C#/Lean) → check → 0 diffs
oxidtr-split.als → generate → extract → structural + expression match with original
oxidtr-split.als → generate (all languages) → extract (multi-lang merge) → unified Alloy model
```

## Development

```bash
cargo test              # 909 tests
cargo run -- generate models/oxidtr-split.als --target rust --output generated
cargo run -- check --model models/oxidtr-split.als --impl generated
cargo run -- extract generated/
```

## License

MIT
