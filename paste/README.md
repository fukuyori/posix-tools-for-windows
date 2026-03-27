# paste - POSIX準拠ファイル結合コマンド（Rust実装）

Windows環境向けのPOSIX準拠`paste`コマンドのRust実装です。GNU拡張も含みます。

## 特徴

- **POSIX準拠**: 標準的な`paste`コマンドの動作を再現
- **GNU拡張対応**: ゼロ終端モード（-z）
- **日本語対応**: ヘルプ・エラーメッセージは日本語
- **柔軟なデリミタ**: 複数文字のデリミタを循環使用可能
- **Windowsでもglob展開**: `cmd.exe` / PowerShell 経由でも `*.txt` などを内部展開
- **高速**: Rustによる効率的な実装

## インストール

```bash
cargo build --release
cp target/release/paste.exe ~/.local/bin/  # または任意のPATHディレクトリ
```

## 使用法

```
paste [オプション]... [ファイル]...
```

複数のファイルの対応する行を結合して出力します。

### オプション

| オプション | 説明 |
|-----------|------|
| `-d, --delimiters=LIST` | デリミタを指定（デフォルト：タブ） |
| `-s, --serial` | ファイルごとに全行を1行にまとめる |
| `-z, --zero-terminated` | 行末をNUL文字として扱う（GNU拡張） |

### デリミタの特殊文字

| 文字 | 意味 |
|------|------|
| `\n` | 改行 |
| `\t` | タブ |
| `\\` | バックスラッシュ |
| `\0` | 空文字（デリミタなし） |

## 使用例

### 基本的な使用法
```bash
# 2つのファイルをタブで結合
paste file1.txt file2.txt

# 3つ以上のファイルも可能
paste file1.txt file2.txt file3.txt

# Windowsでも内部でglob展開
paste *.txt
```

### glob展開のルール
```bash
# マッチしたファイルを名前順で展開
paste logs\\*.txt

# マッチしない場合はリテラルのまま扱う
paste no-match-*.txt

# 隠しファイルは明示しない限り含めない
paste .*.txt
```

- `*`, `?`, `[...]` を POSIX の pathname expansion に近いルールで展開します
- マッチ結果は安定した順序になるよう名前順で処理します
- `*.txt` は `.hidden.txt` に一致しません

### カスタムデリミタ
```bash
# カンマで結合
paste -d, file1.txt file2.txt

# コロンで結合
paste -d: file1.txt file2.txt

# 複数デリミタを循環使用
paste -d',;:' file1.txt file2.txt file3.txt file4.txt
# → file1,file2;file3:file4
```

### シリアルモード（-s）
```bash
# 1ファイルの全行を1行にまとめる
paste -s file.txt
# a1  a2  a3  a4  a5

# カンマ区切りで1行に
paste -s -d, file.txt
# a1,a2,a3,a4,a5

# 複数ファイルをそれぞれ1行ずつに
paste -s file1.txt file2.txt
```

### 標準入力の使用
```bash
# 入力を2列に整形
seq 1 6 | paste - -
# 1    2
# 3    4
# 5    6

# 入力を3列に整形
seq 1 9 | paste - - -
# 1    2    3
# 4    5    6
# 7    8    9

# ファイルと標準入力を組み合わせ
cat numbers.txt | paste file1.txt -
```

### 特殊なデリミタ
```bash
# デリミタなし（連結）
paste -d'\0' file1.txt file2.txt

# 改行をデリミタに（各行を交互に出力）
paste -d'\n' file1.txt file2.txt
```

### 実用例

#### CSVの列を結合
```bash
# 名前と年齢のCSVを結合
paste -d, names.txt ages.txt > combined.csv
```

#### 番号付きリストを作成
```bash
seq 1 10 > numbers.txt
paste -d'. ' numbers.txt items.txt
# 1. item1
# 2. item2
# ...
```

#### ファイルの行を横に並べる
```bash
# ログファイルの各行を比較
paste old.log new.log | column -t
```

#### データの変換
```bash
# 1列のデータを3列に変換
cat data.txt | paste - - -

# カンマ区切りで変換
cat data.txt | paste -d, - - -
```

## 注意事項

- ファイルの長さが異なる場合、短いファイルは空として扱われます
- `-`を複数指定すると、標準入力から順番に行を読み取ります
- デリミタは循環して使用されます（3つのデリミタで4ファイルを結合すると、4番目の区切りは1番目のデリミタ）

## ライセンス

MIT

## 関連

このツールは、Windows環境でUnix系コマンドを使いやすくするプロジェクトの一部です。
`cut`コマンドと組み合わせることで、テキストデータの加工に活用できます。

```bash
# 使用例：cutで列を抽出し、pasteで結合
cut -f1 file1.txt > col1.txt
cut -f3 file2.txt > col3.txt
paste col1.txt col3.txt > combined.txt
```
