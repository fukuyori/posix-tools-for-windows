#!/usr/bin/env bash
# =============================================================================
# 04_batch_extra_test.sh — バッチモードの追加テスト
# =============================================================================

FIND="${FIND:-find}"
BASE="$(mktemp -d /tmp/find_batch_test_XXXXXX)"
TMPOUT="$(mktemp -d /tmp/find_batch_out_XXXXXX)"
trap 'rm -rf "$BASE" "$TMPOUT"' EXIT

PASS=0; FAIL=0

ok() { echo "  ✅ PASS: $1"; PASS=$((PASS+1)); }
ng() { echo "  ❌ FAIL: $1"; echo "       期待: $2"; echo "       実際: $3"; FAIL=$((FAIL+1)); }
check() { [[ "$1" -eq "$2" ]] && ok "$3" || ng "$4" "$2" "$1"; }

section() {
    echo; echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "  $1"; echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
}

mkdir -p "$BASE/dir"
for i in $(seq 1 5); do echo "content$i" > "$BASE/dir/file$i.txt"; done

echo "  自作 find: $FIND"
echo "  デモ用ディレクトリ: $BASE"

# =============================================================================
# テスト A: {} + での部分置換はパーサーエラーになる（GNU find と同仕様）
# =============================================================================
section "テスト A: -exec echo prefix-{} + — パーサーがエラーを返す（GNU find 仕様）"

"$FIND" "$BASE/dir" -type f -exec echo "prefix-{}" + >/dev/null 2>&1
EXIT=$?
if [[ $EXIT -ne 0 ]]; then
    ok "prefix-{} + を正しくエラー終了した (exit=$EXIT)"
else
    # エラーメッセージを stderr から取得して表示
    ERR=$("$FIND" "$BASE/dir" -type f -exec echo "prefix-{}" + 2>&1 || true)
    ng "prefix-{} + がエラーにならなかった" "非ゼロ終了" "ゼロ終了"
    echo "    find の出力: $ERR"
fi

# =============================================================================
# テスト B: {} \; では部分置換が動作する（従来通り）
# =============================================================================
section "テスト B: -exec echo prefix-{} \\; — \\; モードでは部分置換が動作する"

OUT="$TMPOUT/tB.txt"
"$FIND" "$BASE/dir" -type f -name "*.txt" -exec echo "prefix-{}" \; > "$OUT"
COUNT=$(grep -c "^prefix-" "$OUT")
check "$COUNT" 5 \
    "prefix-{} \\; で5ファイル分が prefix- 付きで出力された" \
    "prefix-{} \\; の展開件数が不正"
echo "    出力例: $(head -1 "$OUT")"

# =============================================================================
# テスト C: {} + 完全一致は正常動作
# =============================================================================
section "テスト C: -exec cat {} + — 完全一致バッチは正常動作"

OUT="$TMPOUT/tC.txt"
"$FIND" "$BASE/dir" -type f -name "*.txt" -exec cat {} + > "$OUT"
COUNT=$(wc -l < "$OUT" | tr -d ' ')
check "$COUNT" 5 \
    "-exec cat {} + で5行出力された" \
    "-exec cat {} + の行数が不正"

# =============================================================================
# テスト D: ARG_MAX 分割 — 大量ファイルで複数バッチに分割される
# =============================================================================
section "テスト D: 大量ファイルで ARG_MAX 分割が機能する"

mkdir -p "$BASE/many"
for i in $(seq 1 300); do
    touch "$BASE/many/very_long_filename_for_arg_max_test_$(printf '%04d' $i).txt"
done

# exec 呼び出しごとに受け取ったファイル数をログに記録
LOG="$TMPOUT/batch_log.txt"
"$FIND" "$BASE/many" -type f -name "*.txt" \
    -exec sh -c 'echo "$#"' _ {} + > "$LOG"

TOTAL_FILES=$(wc -l < "$LOG" | tr -d ' ')   # 各行 = 1回の exec が受け取ったファイル数
EXEC_CALLS=$(wc -l < "$LOG" | tr -d ' ')    # 行数 = exec 呼び出し回数
SUM_FILES=$(awk '{s+=$1} END{print s}' "$LOG")

echo "    作成ファイル数  : 300"
echo "    exec 呼び出し回数: $EXEC_CALLS"
echo "    合計処理ファイル数: $SUM_FILES"

check "$SUM_FILES" 300 \
    "ARG_MAX 分割で全300ファイルが処理された (${EXEC_CALLS}回の exec)" \
    "ARG_MAX 分割後の合計ファイル数が不正"

# =============================================================================
# 結果サマリー
# =============================================================================
echo
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
TOTAL=$((PASS + FAIL))
printf "  テスト結果: %d/%d PASS\n" "$PASS" "$TOTAL"
[[ $FAIL -eq 0 ]] && echo "  🎉 全テスト通過" || echo "  ⚠️  ${FAIL} テスト失敗"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
exit $FAIL
