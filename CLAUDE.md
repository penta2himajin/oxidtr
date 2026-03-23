# oxidtr

## セットアップ

```bash
# Rust toolchain (mise経由)
mise use -g rust

# ビルド確認
mise exec rust -- cargo build

# テスト実行
mise exec rust -- cargo test
```

Windows環境の注意:
- `mise exec rust -- cargo ...` でcargoを実行（PATHに直接ないため）
- VS Build Tools 2025が必要（MSVCリンカー）
- Git BashのGNU `link.exe` とMSVC `link.exe` の衝突に注意

## コマンド実行

```bash
# ビルド・テスト
mise exec rust -- cargo build          # ビルド
mise exec rust -- cargo test           # 全テスト実行
mise exec rust -- cargo test --test <name>  # 特定テスト

# CLIサブコマンド
mise exec rust -- cargo run -- generate models/oxidtr.als --target rust --output generated
mise exec rust -- cargo run -- generate models/oxidtr.als --target ts --output generated-ts
mise exec rust -- cargo run -- generate models/oxidtr.als --target kt --output generated-kt
mise exec rust -- cargo run -- generate models/oxidtr.als --target java --output generated-java
mise exec rust -- cargo run -- check --model models/oxidtr.als --impl generated
mise exec rust -- cargo run -- mine generated/
mise exec rust -- cargo run -- mine src/ --lang rust
```

## アーキテクチャ

```
Alloy (.als) → Parser → AST → Lowering → IR → Backend → Generated code
                                            ↓
                                      Constraint Analyzer
                                            ↓
                              Fixtures / Validators / Tests / Schema
```

### モジュール構成

```
src/
  parser/           Alloyパーサー (lexer, ast, parser)
  ir/               中間表現 (nodes, lowering)
  analyze/          制約分析 (constraint info, guarantee, bean validation)
  backend/
    rust/           Rust backend + expr_translator
    typescript/     TS backend + expr_translator
    jvm/            共通JVM層 + Kotlin/Java backends + expr_translator
    schema.rs       JSON Schema生成
  generate.rs       generateパイプライン
  check/            構造的整合性検証 (differ, impl_parser)
  mine/             逆抽出 (rust/ts/kotlin/java/schema extractors, renderer)
```

## 技術スタック

- Rust (oxidtr自体の実装言語)
- 依存: clap 4 (CLI), tempfile (dev)
- 外部依存最小: serde不使用、tree-sitter不使用、パーサーは全て手書き

## 設計原則

- **全コマンドが決定的** — AI非依存、確率的要素なし
- **モデルが唯一の信頼源** — 型・テスト・検証は全てAlloyモデルから導出
- **保証の総量は一定** — 型が強い言語はテスト減、弱い言語はテスト増
- **最小依存** — oxidtr自身がAlloyモデルでセルフホスト可能であること
- **can_guarantee_by_type** — 言語の型の強さに応じてテスト生成量を自動調整

## 開発ワークフロー

- main直push方式（現状）
- CIパス必須: `cargo test` + セルフホスト generate/check (全4言語) + mine round-trip
- コミットは各ステップの動作確認後に行う
- zero warnings ポリシー

### TDD (テスト駆動開発)

機能追加・修正は Red-Green-Refactoring のサイクルに従う:

1. **Red**: 失敗するテストを先に書く
2. **Green**: テストを通す最小限のコードを実装する
3. **Refactor**: テストがグリーンの状態でコードを整理する

### セルフホスト検証

oxidtr自身のドメインモデル `models/oxidtr.als` を使った検証:

```bash
# 全4言語で生成→check→0 diff
cargo run -- generate models/oxidtr.als --target rust --output generated
cargo run -- check --model models/oxidtr.als --impl generated

# mine round-trip
cargo run -- mine generated/ -o /tmp/mined.als
```

### コミット規約

- 各段階でテスト全パス + warning ゼロを確認してからコミット
- コミットメッセージ末尾に `Co-Authored-By: Claude Opus 4.6 (1M context) <noreply@anthropic.com>`
- 大規模変更はブランチまたは段階的コミット

## テスト構成

### ユニットテスト (`cargo test` で常に実行)

| テストファイル | 対象 |
|---|---|
| `parser_sig`, `parser_expr` | Alloyパーサー |
| `lowering` | AST→IR変換 |
| `expr_translator` | 式変換 (Rust) |
| `backend_rust`, `backend_ts`, `backend_jvm` | 各言語コード生成 |
| `test_generation`, `tc_generation` | テスト・TC関数生成 |
| `generate_pipeline` | E2Eパイプライン + 警告検出 |
| `check` | 構造的整合性検証 |
| `analyze`, `enrich` | 制約分析・enrichment |
| `guarantee_differentiation` | 言語間テスト生成差異化 |

### セルフホスト検証 (`cargo test` で常に実行、外部依存なし)

| テストファイル | 対象 |
|---|---|
| `self_hosting` | パース・lower・生成・内容検査・mine sig coverage |
| `self_host_guarantees` | fact→テスト変換・cross-testマーカー・mine/check整合性 |
| `round_trip`, `round_trip_jvm`, `round_trip_enriched` | ラウンドトリップ検証 |
| `commentless_round_trip`, `lossless_round_trip` | コメントなし逆変換 |
| `mine_rust`, `mine_ts` | mine抽出 (言語別) |
| `mine_auto_detect`, `mine_multi_lang` | mine自動検出・multi-lang merge |
| `mine_new_patterns`, `mine_general_patterns` | 一般コードパターン抽出 |

### 対象検証テスト (外部ツールチェイン依存)

| テストファイル | 対象 | 必要ツール |
|---|---|---|
| `target_validation::rust_self_hosted_crate_compiles` | Rust型検査 | cargo |
| `target_validation::rust_self_hosted_tests_pass` | Rustテスト実行 | cargo |
| `target_validation::ts_self_hosted_tests_pass` | TSテスト実行 | bun |
| `target_validation::kotlin_self_hosted_tests_pass` | Kotlinテスト実行 | gradle |
| `target_validation::java_self_hosted_tests_pass` | Javaテスト実行 (ignore) | gradle |

## ロードマップ

`local_docs/oxidtr-spec.md` の実装計画セクションを参照。

主要な未実装:
- Phase 7-10: Swift / Go / C# / Lean backends
- Phase 11-13: Alloy 6 時相拡張 (var/always/eventually)
- explore: Alloyインスタンス異常パターン検出
- cover: カバレッジ×fact直交テスト生成

## Alloyモデルへのフィードバック

コード生成やテストの改善に伴い `models/oxidtr.als` も更新すること:
- 新しいsig/field追加時はモデルにも反映
- 生成物の構造変更はcheck self-hostingで検証
- 警告ゼロを維持（UNREFERENCED_SIG等）
