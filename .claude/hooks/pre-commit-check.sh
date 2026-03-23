#!/usr/bin/env bash
# Pre-commit Test Gate — Claude Code PreToolUse Hook (Bash matcher)
#
# git commit の実行前に全テストとビルドwarningチェックを行い、
# 失敗していればコミットをブロックする。

set -uo pipefail

INPUT=$(cat)
COMMAND=$(echo "$INPUT" | grep -o '"command":"[^"]*"' | sed 's/"command":"//;s/"$//' 2>/dev/null || true)

# git commit 以外は許可
echo "$COMMAND" | grep -q 'git.*commit' || exit 0

echo "Pre-commit: テスト実行中..." >&2

cd "$(git rev-parse --show-toplevel 2>/dev/null || echo .)"

# Build warning check
WARNINGS=$(mise exec rust -- cargo build 2>&1 | grep "^warning:" | grep -v "generated" | wc -l)
if [ "$WARNINGS" -gt 0 ]; then
  echo "⚠ Pre-commit: ビルドwarningが${WARNINGS}件あります。コミットをブロックします。" >&2
  exit 1
fi

# Test check (j=1 for memory safety)
if ! mise exec rust -- cargo test -j 1 2>&1 | tail -1 | grep -q "test result: ok\|^$"; then
  # Check for any failures
  FAILURES=$(mise exec rust -- cargo test -j 1 2>&1 | grep "FAILED" | head -3)
  if [ -n "$FAILURES" ]; then
    echo "⚠ Pre-commit: テストが失敗しています。コミットをブロックします。" >&2
    echo "$FAILURES" >&2
    exit 1
  fi
fi

echo "✓ Pre-commit: 全テストパス、warningゼロ — コミットを許可します。" >&2
exit 0
