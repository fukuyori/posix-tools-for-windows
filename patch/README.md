# patch - POSIX準拠パッチ適用コマンド（Rust実装）

Windows環境向けのPOSIX準拠`patch`コマンドのRust実装です。GNU拡張も含みます。

## 特徴

- **POSIX準拠**: 標準的な`patch`コマンドの動作を再現
- **GNU拡張対応**: ユニファイド形式、コンテキスト形式、通常形式を自動検出
- **日本語対応**: ヘルプ・エラーメッセージは日本語、エンコーディング自動検出
- **多機能**: バックアップ、ドライラン、逆パッチ、ファズマッチング
- **高速**: Rustによる効率的な実装

## インストール

```bash
cargo build --release
cp target/release/patch.exe ~/.local/bin/  # または任意のPATHディレクトリ
```

## 使用法

```
patch [オプション]... [入力ファイル [パッチファイル]]
```

### 入力オプション

| オプション | 説明 |
|-----------|------|
| `-i, --input=FILE` | パッチファイルを指定（デフォルト：標準入力） |
| `-p, --strip=NUM` | パスから先頭NUM個のコンポーネントを削除 |
| `-d, --directory=DIR` | DIRに移動してからパッチを適用 |

### 出力オプション

| オプション | 説明 |
|-----------|------|
| `-o, --output=FILE` | 結果をFILEに出力 |
| `-b, --backup` | 変更前にバックアップを作成 |
| `-z, --suffix=SUFFIX` | バックアップサフィックス（デフォルト：.orig） |

### 動作オプション

| オプション | 説明 |
|-----------|------|
| `-R, --reverse` | パッチを逆適用 |
| `-N, --forward` | 既に適用済みのパッチは無視 |
| `-f, --force` | 確認なしで実行 |
| `-F, --fuzz=NUM` | ファズファクター（デフォルト：2） |
| `--dry-run` | 実際には変更せず確認のみ |
| `--verbose` | 詳細情報を表示 |
| `-s, --silent` | 最小限の出力 |

### 終了ステータス

- `0`: パッチが正常に適用された
- `1`: 一部のハンクが失敗した
- `2`: エラー発生

## 使用例

### 基本的なパッチ適用
```bash
# 標準入力からパッチを適用
patch < fix.patch

# パッチファイルを指定
patch -i fix.patch

# 入力ファイルとパッチファイルを指定
patch file.txt fix.patch
```

### パスストリップ（-p）
```bash
# diff -u a/src/file.c b/src/file.c のようなパッチの場合
# 先頭の a/ や b/ を削除
patch -p1 < fix.patch

# a/b/c/file.c → file.c
patch -p3 < fix.patch
```

### バックアップ
```bash
# .orig ファイルを作成
patch -b < fix.patch

# カスタムサフィックス
patch -b -z .bak < fix.patch
```

### 逆パッチ（変更を元に戻す）
```bash
# パッチを逆適用
patch -R < fix.patch

# バックアップ付きで逆適用
patch -R -b < fix.patch
```

### ドライラン（確認のみ）
```bash
# 実際には変更せず、適用可能か確認
patch --dry-run < fix.patch
```

### ディレクトリ指定
```bash
# 指定ディレクトリに移動してからパッチ適用
patch -d /path/to/source -p1 < fix.patch
```

### 出力ファイル指定
```bash
# 元ファイルを変更せず、別ファイルに出力
patch -o patched.txt original.txt < fix.patch
```

## diffとの連携

```bash
# パッチの作成
diff -u original.txt modified.txt > changes.patch

# パッチの適用
patch < changes.patch

# パッチの逆適用（元に戻す）
patch -R < changes.patch
```

## サポートするパッチ形式

### ユニファイド形式（-u）
```diff
--- file.txt.orig
+++ file.txt
@@ -1,3 +1,4 @@
 line1
-old line
+new line
 line3
+added line
```

### コンテキスト形式（-c）
```
*** file.txt.orig
--- file.txt
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

### 通常形式
```
2c2
< old line
---
> new line
3a4
> added line
```

## 注意事項

- パッチ形式は自動検出されます
- ファズファクター（-F）で多少のずれを許容できます
- 標準入力からパッチを読む場合、入力ファイルを引数で指定してください
- Windows ではシェルが `*.patch` などを展開しないため、この実装が内部で glob 展開を行い、Linux に近い挙動に寄せます
- Windows 専用実装として、ファイルパスの解決では大文字小文字を区別しません
- ed形式のパッチは現在サポートされていません

## ライセンス

MIT

## 関連

このツールは、Windows環境でUnix系コマンドを使いやすくするプロジェクトの一部です。
`diff`コマンドと組み合わせることで、ソースコード管理やファイル同期に活用できます。

```bash
# ワークフロー例
diff -u old_version/ new_version/ > update.patch
patch -p1 -d target_directory/ < update.patch
```
