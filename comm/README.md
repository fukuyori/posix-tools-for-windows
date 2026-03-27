# comm - POSIX準拠ファイル比較コマンド（Rust実装）

Windows環境向けのPOSIX準拠`comm`コマンドのRust実装です。GNU拡張も含み、Windows ではシェルが行わない glob 展開も補います。

## 特徴

- **POSIX準拠**: 標準的な`comm`コマンドの動作を再現
- **GNU拡張対応**: 大文字小文字無視、カスタムデリミタ、合計表示など
- **日本語対応**: ヘルプ・エラーメッセージは日本語
- **柔軟な出力制御**: 列の表示/非表示を自由に制御
- **高速**: Rustによる効率的な実装
- **Windows向けglob補完**: `*.txt` のような未展開引数を POSIX シェルに近い形で扱う

## インストール

```bash
cargo build --release
cp target/release/comm.exe ~/.local/bin/  # または任意のPATHディレクトリ
```

## 使用法

```
comm [オプション]... ファイル1 ファイル2
```

**重要**: 入力ファイルはソート済みである必要があります。

### デフォルト出力

デフォルトでは3列で出力されます：
- 列1: file1にのみある行
- 列2: file2にのみある行
- 列3: 両方にある行（共通行）

### POSIXオプション

| オプション | 説明 |
|-----------|------|
| `-1` | file1にのみある行を出力しない |
| `-2` | file2にのみある行を出力しない |
| `-3` | 両方にある行を出力しない |

### GNU拡張オプション

| オプション | 説明 |
|-----------|------|
| `-i, --ignore-case` | 大文字小文字を無視して比較 |
| `-z, --zero-terminated` | 行末をNUL文字として扱う |
| `--check-order` | 入力がソートされているかチェック |
| `--nocheck-order` | ソートチェックを無効化 |
| `--output-delimiter=STR` | 列の区切り文字を指定 |
| `--total` | 各列の合計を表示 |

## 使用例

### 基本的な使用法
```bash
# 3列すべてを表示
comm file1.txt file2.txt

# 出力例:
# apple          ← file1のみ
#     banana     ← file2のみ
#         cherry ← 両方にある
```

### よく使うパターン

```bash
# 共通行のみ表示（両方にある行）
comm -12 file1.txt file2.txt

# file1のみにある行を表示
comm -23 file1.txt file2.txt

# file2のみにある行を表示
comm -13 file1.txt file2.txt

# どちらか一方にのみある行を表示
comm -3 file1.txt file2.txt
```

### 大文字小文字を無視
```bash
# Apple と apple を同一として扱う
comm -i file1.txt file2.txt
```

### カスタムデリミタ
```bash
# カンマ区切りで出力
comm --output-delimiter=',' file1.txt file2.txt

# パイプ区切り
comm --output-delimiter='|' file1.txt file2.txt
```

### 合計表示
```bash
# 各列の行数を表示
comm --total file1.txt file2.txt
# 出力の最後に:
# 2    3    5    total
```

### 実用例

#### 2つのリストの差分を確認
```bash
# ソートしてから比較
sort list1.txt > sorted1.txt
sort list2.txt > sorted2.txt
comm sorted1.txt sorted2.txt
```

#### 新規追加されたファイルを確認
```bash
ls old_dir | sort > old_files.txt
ls new_dir | sort > new_files.txt
comm -13 old_files.txt new_files.txt  # 新規追加のみ
```

#### 削除されたファイルを確認
```bash
comm -23 old_files.txt new_files.txt  # 削除されたもののみ
```

#### 2つのCSVの共通IDを抽出
```bash
cut -d, -f1 data1.csv | sort > ids1.txt
cut -d, -f1 data2.csv | sort > ids2.txt
comm -12 ids1.txt ids2.txt  # 共通ID
```

#### 重複行の確認
```bash
sort file.txt | uniq > unique.txt
sort file.txt > all.txt
comm -23 all.txt unique.txt  # 重複していた行
```

## 出力の読み方

```
apple           ← 列1: file1のみ（インデントなし）
    banana      ← 列2: file2のみ（タブ1つ）
        cherry  ← 列3: 共通（タブ2つ）
date
    fig
        grape
```

`-1`, `-2`, `-3`オプションで列を非表示にすると、インデントも調整されます。

## 注意事項

- **入力はソート済み必須**: ソートされていないファイルを渡すと結果が正しくなりません
- 標準入力は`-`で指定できます（片方のみ）
- ロケールによってソート順が異なる場合があります（`LC_ALL=C`を推奨）
- Windows では未展開の glob 引数をプログラム側で展開します。マッチしない場合は一般的な POSIX シェルと同様にリテラルのまま扱います

## ライセンス

MIT

## 関連

このツールは、Windows環境でUnix系コマンドを使いやすくするプロジェクトの一部です。
`sort`、`uniq`コマンドと組み合わせることで、テキストデータの比較・分析に活用できます。

```bash
# ワークフロー例
sort file1.txt > sorted1.txt
sort file2.txt > sorted2.txt
comm -12 sorted1.txt sorted2.txt > common.txt
```
