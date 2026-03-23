#!/usr/bin/env bash
# TDD Green Phase Feedback — Claude Code PostToolUse Hook
#
# src/ 配下のファイル編集後に関連テストを実行し結果をフィードバック。
# ブロックしない（情報提供のみ）。

set -uo pipefail

INPUT=$(cat)
FILE_PATH=$(echo "$INPUT" | grep -o '"file_path":"[^"]*"' | sed 's/"file_path":"//;s/"$//' 2>/dev/null || true)

[ -z "$FILE_PATH" ] && exit 0
echo "$FILE_PATH" | grep -q '\.rs$' || exit 0

MODULE=$(basename "$FILE_PATH" .rs)
[ "$MODULE" = "mod" ] && MODULE=$(basename "$(dirname "$FILE_PATH")")

PROJECT_DIR=$(echo "$FILE_PATH" | sed 's|/src/.*||;s|/tests/.*||')
RELATED_TESTS=$(find "$PROJECT_DIR/tests" -name "*.rs" 2>/dev/null | xargs grep -l "$MODULE" 2>/dev/null | head -3)

[ -z "$RELATED_TESTS" ] && exit 0

FIRST_TEST=$(echo "$RELATED_TESTS" | head -1)
TEST_NAME=$(basename "$FIRST_TEST" .rs)

cd "$PROJECT_DIR"
RESULT=$(mise exec rust -- cargo test --test "$TEST_NAME" 2>&1 | tail -3)

if echo "$RESULT" | grep -q "test result: ok"; then
  echo "✓ Green: $TEST_NAME 全パス" >&2
else
  FAILED=$(echo "$RESULT" | grep -o '[0-9]* failed' | head -1)
  echo "✗ Red: $TEST_NAME — ${FAILED:-テスト失敗}" >&2
fi

exit 0
