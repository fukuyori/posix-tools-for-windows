# diff - POSIX準拠ファイル比較コマンド（Rust実装）

Windows環境向けのPOSIX準拠`diff`コマンドのRust実装です。GNU拡張も含みます。

## 特徴

- **POSIX準拠**: 標準的な`diff`コマンドの動作を再現
- **GNU拡張対応**: ユニファイド形式、コンテキスト形式、横並び表示など
- **日本語対応**: ヘルプ・エラーメッセージは日本語、エンコーディング自動検出
- **複数出力形式**: 通常、ユニファイド、コンテキスト、ed、RCS、横並び
- **高速**: Rustによる効率的なLCSアルゴリズム実装

## インストール

```bash
cargo build --release
cp target/release/diff.exe ~/.local/bin/  # または任意のPATHディレクトリ
```

## 使用法

```
diff [オプション]... ファイル1 ファイル2
```

### 出力形式オプション

| オプション | 説明 |
|-----------|------|
| (なし) | 通常形式（ed風） |
| `-c, -C NUM` | コンテキスト形式（デフォルト3行） |
| `-u, -U NUM` | ユニファイド形式（デフォルト3行） |
| `-e, --ed` | edスクリプト形式 |
| `-n, --rcs` | RCS形式 |
| `-y, --side-by-side` | 横並び表示 |
| `-W NUM` | 横並び時の幅（デフォルト130） |
| `--suppress-common-lines` | 横並び時に共通行を非表示 |

### 比較オプション

| オプション | 説明 |
|-----------|------|
| `-i, --ignore-case` | 大文字小文字を無視 |
| `-b, --ignore-space-change` | 空白の量の変化を無視 |
| `-w, --ignore-all-space` | すべての空白を無視 |
| `-B, --ignore-blank-lines` | 空行を無視 |
| `-Z, --ignore-trailing-space` | 行末の空白を無視 |

### その他のオプション

| オプション | 説明 |
|-----------|------|
| `-q, --brief` | 異なるかどうかのみ報告 |
| `-s, --report-identical-files` | 同一ファイルを報告 |
| `-r, --recursive` | ディレクトリを再帰比較 |
| `-N, --new-file` | 存在しないファイルを空として扱う |
| `-t, --expand-tabs` | タブをスペースに展開 |
| `--label=LABEL` | ファイルラベルを指定 |

### 終了ステータス

- `0`: ファイルが同一
- `1`: ファイルが異なる
- `2`: エラー発生

## 使用例

### 基本的な比較
```bash
# 通常形式
diff file1.txt file2.txt

# ユニファイド形式（パッチファイル作成に最適）
diff -u file1.txt file2.txt

# コンテキスト形式
diff -c file1.txt file2.txt
```

### 横並び比較
```bash
# 横並びで表示
diff -y file1.txt file2.txt

# 幅を指定
diff -y -W 100 file1.txt file2.txt

# 共通行を非表示
diff -y --suppress-common-lines file1.txt file2.txt
```

### 空白・大文字小文字の無視
```bash
# 大文字小文字を無視
diff -i file1.txt file2.txt

# 空白の変化を無視
diff -b file1.txt file2.txt

# すべての空白を無視
diff -w file1.txt file2.txt
```

### ディレクトリの比較
```bash
# 再帰的に比較
diff -r dir1 dir2

# 新規ファイルも比較
diff -rN dir1 dir2
```

### パッチファイルの作成
```bash
# ユニファイド形式でパッチを作成
diff -u original.txt modified.txt > changes.patch

# パッチを適用（patchコマンド使用）
patch < changes.patch
```

### 簡易比較
```bash
# 異なるかどうかのみ確認
diff -q file1.txt file2.txt

# 同一ファイルも報告
diff -sq file1.txt file2.txt
```

## 出力形式の例

### 通常形式
```
2c2
< old line
---
> new line
3a4
> added line
```

### ユニファイド形式
```diff
--- file1.txt
+++ file2.txt
@@ -1,3 +1,4 @@
 line1
-old line
+new line
 line3
+added line
```

### コンテキスト形式
```
*** file1.txt
--- file2.txt
***************
*** 1,3 ****
  line1
- old line
  line3
--- 1,4 ----
  line1
+ new line
  line3
+ added line
```

### 横並び形式
```
line1                              line1
old line                         | new line
line3                              line3
                                 > added line
```

## 注意事項

- ディレクトリ比較では `-r` オプションが必要です
- バイナリファイルの比較は行ベースの比較となります
- 大きなファイルでは処理に時間がかかる場合があります

## ライセンス

MIT

## 関連

このツールは、Windows環境でUnix系コマンドを使いやすくするプロジェクトの一部です。
`patch`コマンドと組み合わせることで、ソースコード管理やファイル同期に活用できます。
