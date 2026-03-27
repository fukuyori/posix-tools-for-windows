# join - POSIX準拠ファイル結合コマンド（Rust実装）

Windows環境向けのPOSIX準拠`join`コマンドのRust実装です。GNU拡張も含みます。

## 特徴

- **POSIX準拠**: 標準的な`join`コマンドの動作を再現
- **GNU拡張対応**: 大文字小文字無視、ヘッダ、ソートチェックなど
- **日本語対応**: ヘルプ・エラーメッセージは日本語
- **柔軟な結合**: 内部結合、左外部結合、右外部結合、完全外部結合
- **高速**: Rustによる効率的な実装

## インストール

```bash
cargo build --release
cp target/release/join.exe ~/.local/bin/  # または任意のPATHディレクトリ
```

## 使用法

```
join [オプション]... ファイル1 ファイル2
```

**重要**: 入力ファイルは結合フィールドでソート済みである必要があります。

### Windows での glob 展開

Windows の `cmd.exe` や PowerShell は、POSIX 系シェルのように `*.txt` を自動で pathname 展開しません。
この `join` は Windows 上では位置引数だけを内部で glob 展開し、Linux でシェル展開された場合に近い形で扱います。

- `join sorted-*.txt other.txt`
  Windows でも `sorted-*.txt` を内部で展開してから処理します
- glob が 1 件も見つからない場合
  パターン文字列をそのままファイル名として扱います
- glob が複数件に一致した場合
  Linux でシェル展開されたときと同様に追加の位置引数が発生したものとして扱われ、`join` は「引数が多すぎます」で終了します

Linux / macOS では通常どおりシェルが先に展開するため、この内部展開は実質的に Windows 向けの補完です。

### POSIXオプション

| オプション | 説明 |
|-----------|------|
| `-1 FIELD` | file1の結合フィールド（デフォルト：1） |
| `-2 FIELD` | file2の結合フィールド（デフォルト：1） |
| `-j FIELD` | 両方のファイルの結合フィールド |
| `-t CHAR` | フィールドセパレータ（デフォルト：空白） |
| `-o FORMAT` | 出力フォーマットを指定 |
| `-e STRING` | 空フィールドの置換文字列 |
| `-a FILENUM` | マッチしない行も出力（1 or 2） |
| `-v FILENUM` | マッチしない行のみ出力（1 or 2） |

### GNU拡張オプション

| オプション | 説明 |
|-----------|------|
| `-i, --ignore-case` | 大文字小文字を無視 |
| `-z, --zero-terminated` | NUL終端モード |
| `--check-order` | ソート順をチェック |
| `--nocheck-order` | ソートチェック無効化 |
| `--header` | 最初の行をヘッダとして扱う |

### 出力フォーマット（-o）

| 指定 | 意味 |
|------|------|
| `0` | 結合フィールド |
| `1.N` | file1のN番目のフィールド |
| `2.N` | file2のN番目のフィールド |

## 使用例

### 基本的な結合（内部結合）
```bash
# 最初のフィールドで結合
join file1.txt file2.txt

# file1:           file2:           結果:
# 1 apple          1 100            1 apple 100
# 2 banana         2 200            2 banana 200
# 3 cherry         4 400
```

### 異なるフィールドで結合
```bash
# file1の2列目とfile2の1列目で結合
join -1 2 -2 1 file1.txt file2.txt

# file1の3列目とfile2の2列目で結合
join -1 3 -2 2 file1.txt file2.txt
```

### CSVファイルの結合
```bash
# カンマ区切り
join -t, file1.csv file2.csv

# タブ区切り
join -t$'\t' file1.tsv file2.tsv
```

### 外部結合
```bash
# 左外部結合（file1のマッチしない行も出力）
join -a 1 file1.txt file2.txt

# 右外部結合（file2のマッチしない行も出力）
join -a 2 file1.txt file2.txt

# 完全外部結合（両方のマッチしない行も出力）
join -a 1 -a 2 file1.txt file2.txt
```

### マッチしない行のみ
```bash
# file1でマッチしなかった行のみ
join -v 1 file1.txt file2.txt

# file2でマッチしなかった行のみ
join -v 2 file1.txt file2.txt
```

### 出力フォーマット指定
```bash
# 結合フィールド、file1の2列目、file2の2列目
join -o 0,1.2,2.2 file1.txt file2.txt

# file1の全フィールド、file2の2列目
join -o 1.1,1.2,1.3,2.2 file1.txt file2.txt
```

### 空フィールドの置換
```bash
# マッチしないフィールドを"N/A"で置換
join -a 1 -a 2 -e "N/A" -o 0,1.2,2.2 file1.txt file2.txt

# 結果:
# 1 apple 100
# 2 banana 200
# 3 cherry N/A
# 4 N/A 400
```

### 大文字小文字を無視
```bash
# "Apple"と"apple"をマッチさせる
join -i file1.txt file2.txt
```

### ヘッダ付きファイル
```bash
# 最初の行はヘッダとして扱う
join --header file1.txt file2.txt
```

### Windows での glob 使用例
```powershell
# Windows でも sorted-*.txt を内部展開
join sorted-*.txt other.txt

# 一致がない場合はリテラルのまま扱われる
join missing-*.txt other.txt
```

## 実用例

### データベース風の結合
```bash
# 社員IDで社員情報と給与情報を結合
sort -k1 employees.txt > sorted_emp.txt
sort -k1 salaries.txt > sorted_sal.txt
join sorted_emp.txt sorted_sal.txt
```

### ログ分析
```bash
# IPアドレスでアクセスログと地域情報を結合
sort access.log | cut -d' ' -f1 | uniq > ips.txt
join -t' ' ips.txt geoip.txt
```

### 差分抽出
```bash
# 新しいリストにのみあるエントリを抽出
sort old_list.txt > old_sorted.txt
sort new_list.txt > new_sorted.txt
join -v 2 old_sorted.txt new_sorted.txt
```

## 注意事項

- **入力はソート済み必須**: 結合フィールドでソートされていない場合、結果は正しくありません
- 標準入力は`-`で指定できます（片方のみ）
- Windows では位置引数の glob を内部展開し、POSIX 系シェルに近い形で扱います
- Windows で glob が複数ファイルに一致した場合は、Linux でシェル展開されたときと同様に追加引数扱いとなり、`join` はエラーになります
- ロケールによってソート順が異なる場合があります（`LC_ALL=C`を推奨）
- 同一キーを持つ複数行がある場合、デカルト積で結合されます

## ライセンス

MIT

## 関連

このツールは、Windows環境でUnix系コマンドを使いやすくするプロジェクトの一部です。
`sort`、`cut`コマンドと組み合わせることで、データの結合・分析に活用できます。

```bash
# ワークフロー例
cut -f1,3 data1.txt | sort > prepared1.txt
cut -f1,2 data2.txt | sort > prepared2.txt
join prepared1.txt prepared2.txt > merged.txt
```
