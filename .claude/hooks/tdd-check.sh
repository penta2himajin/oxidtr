#!/usr/bin/env bash
# TDD Red Phase Check — Claude Code PreToolUse Hook
#
# src/ 配下の非テストファイルを編集する前に、関連テストが
# 「失敗している（Red）」ことを確認する。
#
# - 関連テストが全パス → ブロック（先にテストを追加/修正せよ）
# - 関連テストが失敗あり → 許可（Red Phase 確認済み）
# - 関連テストなし → 許可

set -euo pipefail

INPUT=$(cat)
FILE_PATH=$(echo "$INPUT" | grep -o '"file_path":"[^"]*"' | sed 's/"file_path":"//;s/"$//' 2>/dev/null || true)

# file_path がなければ許可
[ -z "$FILE_PATH" ] && exit 0

# src/ 配下の .rs ファイルのみ対象（テストファイルは除外）
echo "$FILE_PATH" | grep -q '/src/' || exit 0
echo "$FILE_PATH" | grep -q '/tests/' && exit 0

# 変更されたモジュール名を推定
MODULE=$(basename "$FILE_PATH" .rs)
# mod.rs の場合は親ディレクトリ名
[ "$MODULE" = "mod" ] && MODULE=$(basename "$(dirname "$FILE_PATH")")

# 関連テストファイルを探す
PROJECT_DIR=$(echo "$FILE_PATH" | sed 's|/src/.*||')
RELATED_TESTS=$(find "$PROJECT_DIR/tests" -name "*.rs" 2>/dev/null | xargs grep -l "$MODULE" 2>/dev/null | head -5)

[ -z "$RELATED_TESTS" ] && {
  echo "ℹ TDD: 関連テストが見つかりません: $MODULE" >&2
  exit 0
}

# 関連テストを実行
FIRST_TEST=$(echo "$RELATED_TESTS" | head -1)
TEST_NAME=$(basename "$FIRST_TEST" .rs)

cd "$PROJECT_DIR"
if mise exec rust -- cargo test --test "$TEST_NAME" 2>&1 | grep -q "test result: ok"; then
  echo "⚠ TDD: 関連テスト ($TEST_NAME) が全パスしています。先にfailingテストを書いてください。" >&2
  exit 1
else
  echo "✓ TDD: Red Phase 確認済み ($TEST_NAME に失敗テストあり)" >&2
  exit 0
fi
