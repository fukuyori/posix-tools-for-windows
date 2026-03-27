#!/usr/bin/env bash
# テスト A の動作を1ステップずつ確認するデバッグスクリプト
FIND="${FIND:-find}"
BASE=$(mktemp -d /tmp/find_dbg_XXXXXX)
mkdir -p "$BASE/dir"
for i in 1 2 3; do echo "c$i" > "$BASE/dir/file$i.txt"; done
trap 'rm -rf "$BASE"' EXIT

echo "=== 環境情報 ==="
echo "FIND=$FIND"
echo "FIND の実体: $(file "$FIND" 2>/dev/null || echo '不明')"
echo "BASE=$BASE"
echo "bash: $BASH_VERSION"
echo ""

echo "=== Step 1: find に渡す引数を確認 ==="
cat << 'ARGDUMP' > /tmp/argdump.sh
#!/usr/bin/env bash
echo "受け取った引数:"
for a in "$@"; do printf "  [%s]\n" "$a"; done
exit 1
ARGDUMP
chmod +x /tmp/argdump.sh
FIND=/tmp/argdump.sh "$FIND" "$BASE/dir" -type f -exec echo "prefix-{}" + 2>/dev/null || true
# ↑ $FIND を /tmp/argdump.sh で上書きして引数確認
/tmp/argdump.sh "$BASE/dir" -type f -exec echo "prefix-{}" +
echo ""

echo "=== Step 2: 直接呼び出し（-type f あり） ==="
"$FIND" "$BASE/dir" -type f -exec echo "prefix-{}" + 2>&1
echo "exit: $?"
echo ""

echo "=== Step 3: 直接呼び出し（-maxdepth 0 あり） ==="
"$FIND" "$BASE/dir" -maxdepth 0 -exec echo "prefix-{}" + 2>&1
echo "exit: $?"
echo ""

echo "=== Step 4: -type f なし ==="
"$FIND" "$BASE/dir" -exec echo "prefix-{}" + 2>&1
echo "exit: $?"
