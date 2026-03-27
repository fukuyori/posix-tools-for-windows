# uniq - POSIX準拠重複除去コマンド（Rust実装）

Windows環境向けのPOSIX準拠`uniq`コマンドのRust実装です。GNU拡張も含みます。

## 特徴

- **POSIX準拠**: 標準的な`uniq`コマンドの動作を再現
- **GNU拡張対応**: `-D`（全重複表示）、`--group`（グループ表示）など
- **日本語対応**: ヘルプ・エラーメッセージは日本語、エンコーディング自動検出（UTF-8, Shift_JIS, EUC-JP）
- **glob展開**: Windows環境でも`*.txt`などのパターンをシェル展開に近い形で内部展開
- **Windows向けのファイル名解決**: glob時のファイル名比較は大文字小文字を区別しない
- **高速**: Rustによる効率的な実装

## インストール

```bash
cargo build --release
cp target/release/uniq.exe ~/.local/bin/  # または任意のPATHディレクトリ
```

## 使用法

```
uniq [オプション]... [入力ファイル [出力ファイル]]
```

### オプション

| オプション | 説明 |
|-----------|------|
| `-c, --count` | 出現回数を行頭に付加 |
| `-d, --repeated` | 重複している行のみ表示（各グループ1行） |
| `-D` | 重複している行をすべて表示 |
| `--all-repeated[=METHOD]` | `-D`と同様、METHODでグループを区切る |
| `-f N, --skip-fields=N` | 先頭のNフィールドを比較から除外 |
| `-i, --ignore-case` | 大文字小文字を区別しない |
| `-s N, --skip-chars=N` | 先頭のN文字を比較から除外 |
| `-u, --unique` | 重複していない行のみ表示 |
| `-w N, --check-chars=N` | 行の先頭N文字のみ比較 |
| `-z, --zero-terminated` | 行区切りを改行ではなくNULに |
| `--group[=METHOD]` | 空行でグループを区切って表示 |

### --all-repeated のMETHOD

- `none`: 区切りなし（デフォルト）
- `prepend`: 各グループの前に空行
- `separate`: グループ間に空行

### --group のMETHOD

- `separate`: グループ間に空行（デフォルト）
- `prepend`: 各グループの前に空行
- `append`: 各グループの後に空行
- `both`: 前後に空行

## 使用例

### 基本的な重複除去
```bash
uniq file.txt
```

### カウント付き
```bash
uniq -c file.txt
#       2 apple
#       3 banana
#       1 cherry
```

### 重複行のみ表示
```bash
uniq -d file.txt
```

### ユニークな行のみ表示
```bash
uniq -u file.txt
```

### 大文字小文字を無視
```bash
uniq -i file.txt
```

### 特定フィールド以降を比較
```bash
# 最初のフィールド（番号など）を無視
uniq -f1 file.txt
```

### 先頭N文字を無視
```bash
# 先頭3文字をスキップして比較
uniq -s3 file.txt
```

### 先頭N文字のみで比較
```bash
# 先頭5文字だけで重複判定
uniq -w5 file.txt
```

### sort と組み合わせて全重複を除去
```bash
# uniqは隣接行のみ比較するため、事前にソートが必要
sort file.txt | uniq
```

### 出現頻度順に表示
```bash
sort file.txt | uniq -c | sort -rn
```

### グループ表示
```bash
uniq --group file.txt
# apple
# apple
#
# banana
# banana
#
# cherry
```

## 注意事項

`uniq`は**隣接する行のみ**を比較します。ファイル全体の重複を除去するには、先に`sort`でソートしてください。

```bash
# 正しい使い方
sort file.txt | uniq

# これでは離れた位置の重複は除去されない
uniq file.txt
```

## ライセンス

MIT

## 関連

このツールは、Windows環境でUnix系コマンドを使いやすくするプロジェクトの一部です。
`sort`コマンドと組み合わせて使用することで、より強力なテキスト処理が可能になります。
