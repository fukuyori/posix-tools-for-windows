# xargs - POSIX準拠引数展開コマンド（Rust実装）

Windows環境向けのPOSIX準拠`xargs`コマンドのRust実装です。GNU拡張も含みます。

## 特徴

- **POSIX準拠**: 標準的な`xargs`コマンドの動作を再現
- **GNU拡張対応**: `-0`（NUL区切り）、`-P`（並列実行）、`-I`（プレースホルダ）など
- **日本語対応**: ヘルプ・エラーメッセージは日本語
- **クォート対応**: シングル・ダブルクォートで囲まれた引数を正しく処理
- **高速**: Rustによる効率的な実装、並列実行対応

## インストール

```bash
cargo build --release
cp target/release/xargs.exe ~/.local/bin/  # または任意のPATHディレクトリ
```

## 使用法

```
xargs [オプション]... [コマンド [初期引数]...]
```

標準入力から引数を読み込み、コマンドを実行します。

### オプション

| オプション | 説明 |
|-----------|------|
| `-0, --null` | NUL文字を区切りに使用（find -print0と併用） |
| `-a, --arg-file=FILE` | FILEから引数を読み込む |
| `-d, --delimiter=CHAR` | 区切り文字を指定 |
| `-E END` | ENDを入力終了文字列として扱う |
| `-I REPLACE` | REPLACEをプレースホルダとして使用 |
| `-L MAX-LINES` | 1回の実行に最大MAX-LINES行を使用 |
| `-n, --max-args=MAX` | 1回の実行に最大MAX個の引数を渡す |
| `-P, --max-procs=MAX` | 最大MAX個のプロセスを並列実行 |
| `-p, --interactive` | 各実行前に確認 |
| `-r, --no-run-if-empty` | 入力が空なら実行しない |
| `-s, --max-chars=MAX` | コマンドラインの最大長 |
| `-t, --verbose` | 実行コマンドを表示 |
| `-x, --exit` | コマンドライン長超過で終了 |

## 使用例

### 基本的な使い方
```bash
# ファイル一覧をcatに渡す
find . -name "*.txt" | xargs cat

# 引数を2つずつに分けて実行
echo "1 2 3 4 5" | xargs -n 2 echo
# 1 2
# 3 4
# 5
```

### プレースホルダ（-I）
```bash
# 各ファイルのバックアップを作成
find . -name "*.txt" | xargs -I {} cp {} {}.bak

# ファイル名を含むメッセージを表示
ls *.txt | xargs -I {} echo "Processing: {}"
```

### NUL区切り（-0）
```bash
# スペースを含むファイル名を正しく処理
find . -name "*.txt" -print0 | xargs -0 cat

# 安全なファイル削除
find . -name "*.tmp" -print0 | xargs -0 rm -f
```

### 並列実行（-P）
```bash
# 4並列で画像変換
find . -name "*.jpg" | xargs -P 4 -I {} convert {} {}.png

# 10並列でダウンロード
cat urls.txt | xargs -n 1 -P 10 curl -O

# CPUコア数で並列（-P 0）
find . -name "*.c" | xargs -P 0 -I {} gcc -c {}
```

### 詳細表示（-t）
```bash
# 実行コマンドを確認
ls *.txt | xargs -t wc -l
# wc -l file1.txt file2.txt file3.txt
```

### 確認モード（-p）
```bash
# 各実行前に確認
find . -name "*.bak" | xargs -p rm
# rm file1.bak?... y
```

### カスタム区切り文字（-d）
```bash
# コロン区切り
echo "a:b:c:d" | xargs -d : echo
# a b c d

# 改行区切りを明示
cat list.txt | xargs -d '\n' echo
```

### 行ごとの処理（-L）
```bash
# 1行ずつ処理
cat commands.txt | xargs -L 1 sh -c

# 2行ずつ処理
cat pairs.txt | xargs -L 2 echo
```

### 入力ファイル（-a）
```bash
# ファイルから引数を読み込む
xargs -a files.txt rm -f
```

### 終了文字列（-E）
```bash
# ENDで入力終了
printf "a\nb\nEND\nc\n" | xargs -E END echo
# a b
```

## findとの組み合わせ

```bash
# すべてのログファイルを削除
find /var/log -name "*.log" -mtime +30 | xargs rm -f

# 権限を一括変更
find . -type f -name "*.sh" | xargs chmod +x

# 特定パターンを検索
find . -name "*.c" | xargs grep "TODO"
```

## パイプラインでの使用

```bash
# 単語頻度をカウント
cat text.txt | tr -cs '[:alnum:]' '\n' | sort | uniq -c | sort -rn | head -10

# ファイルサイズの合計
find . -name "*.jpg" -print0 | xargs -0 du -ch | tail -1
```

## 注意事項

- コマンドが指定されない場合、`echo`が使用されます
- `-I`オプションは暗黙的に`-L 1`を設定します
- `-P 0`は利用可能なCPUコア数で並列実行します
- クォートで囲まれた引数は1つの引数として扱われます
- Windowsでは`-i`（SIGINT無視）は効果がありません

## ライセンス

MIT

## 関連

このツールは、Windows環境でUnix系コマンドを使いやすくするプロジェクトの一部です。
`find`、`sort`、`uniq`、`tr`と組み合わせて使用することで、強力なバッチ処理が可能になります。

```bash
# ファイル処理パイプライン例
find . -name "*.txt" -print0 | xargs -0 -P 4 -I {} sh -c 'cat {} | sort | uniq > {}.sorted'
```
