# sort - POSIX準拠ソートコマンド（Rust実装）

Windows環境向けのPOSIX準拠`sort`コマンドのRust実装です。GNU拡張も含みます。

## 特徴

- **POSIX準拠**: 標準的な`sort`コマンドの動作を再現
- **GNU拡張対応**: `-h`（人間可読数値）、`-V`（バージョン番号）など
- **日本語対応**: ヘルプ・エラーメッセージは日本語、エンコーディング自動検出（UTF-8, Shift_JIS, EUC-JP）
- **glob展開**: Windows環境でも`*.txt`などのパターンを展開
- **POSIX寄りの既定比較**: 行内容の比較は既定で大文字小文字を区別し、`-f` でのみ非区別
- **Windows向けのファイル名解決**: glob 展開ではファイル名の大文字小文字を区別しない
- **高速**: Rustによる効率的な実装

## インストール

```bash
cargo build --release
cp target/release/sort.exe ~/.local/bin/  # または任意のPATHディレクトリ
```

## 使用法

```
sort [オプション]... [ファイル]...
```

### 順序オプション

| オプション | 説明 |
|-----------|------|
| `-b, --ignore-leading-blanks` | 先頭の空白を無視 |
| `-d, --dictionary-order` | 空白と英数字のみ比較 |
| `-f, --ignore-case` | 大文字小文字を区別しない |
| `-g, --general-numeric-sort` | 一般的な数値としてソート |
| `-h, --human-numeric-sort` | 人間可読数値でソート（2K, 1G等） |
| `-i, --ignore-nonprinting` | 印字可能文字のみ比較 |
| `-M, --month-sort` | 月名でソート（JAN < FEB < ... < DEC） |
| `-n, --numeric-sort` | 文字列を数値としてソート |
| `-r, --reverse` | 逆順にソート |
| `-V, --version-sort` | バージョン番号としてソート |

### その他のオプション

| オプション | 説明 |
|-----------|------|
| `-c, --check` | ソート済みか確認、ソートしない |
| `-C, --check=silent` | `-c`と同様だがエラーメッセージを出さない |
| `-k, --key=KEYDEF` | キー定義に従ってソート |
| `-m, --merge` | ソート済みファイルをマージ（未実装） |
| `-o, --output=FILE` | 結果をFILEに出力 |
| `-s, --stable` | 安定ソート（同値の順序を保持） |
| `-t, --field-separator=SEP` | フィールド区切り文字をSEPに設定 |
| `-u, --unique` | 重複行を削除 |
| `-z, --zero-terminated` | 行区切りを改行ではなくNULに |

### キー定義（KEYDEF）

`F[.C][OPTS][,F[.C][OPTS]]` の形式でソート位置を指定します。

- `F`: フィールド番号（1始まり）
- `C`: フィールド内の文字位置（1始まり）
- `OPTS`: このキーに適用するオプション（bdfiMnhrV）

## 使用例

### 基本ソート
```bash
sort file.txt
```

### 数値ソート
```bash
sort -n numbers.txt
```

### 逆順ソート
```bash
sort -r file.txt
```

### 特定フィールドでソート（CSVなど）
```bash
# 3番目のフィールドを数値としてソート（:区切り）
sort -t: -k3,3n /etc/passwd

# 1番目を文字、2番目を数値でソート
sort -k1,1 -k2,2n data.txt
```

### 重複削除
```bash
sort -u file.txt
```

### 人間可読数値でソート
```bash
# 1K, 2M, 3G などをサイズ順にソート
du -h | sort -h
```

### バージョン番号でソート
```bash
sort -V versions.txt
# 1.1 < 1.2 < 1.9 < 1.10 < 2.0
```

### 複数ファイルをまとめてソート
```bash
sort file1.txt file2.txt file3.txt
```

### glob展開（Windows対応）
```bash
sort *.txt
sort data/*.csv
```

一致するファイルがある場合は内部でglob展開し、一致しない場合はパターン文字列をそのままファイル名として扱います。これは、POSIX系シェルで未展開のワイルドカードがコマンド側へ渡されたときの流れに近づけるためです。Windows 上ではファイル名の解決に合わせて、glob の照合は大文字小文字を区別しません。

### Windows版の挙動

この実装は、行内容の比較については POSIX に近い既定挙動を優先し、大文字小文字を区別します。`A` と `a` を同値として扱いたい場合だけ `-f` を使います。一方で、ファイル名の glob 展開は Windows の慣習に合わせて大文字小文字を区別しません。

### ソート済み確認
```bash
sort -c file.txt && echo "ソート済み"
```

## ライセンス

MIT

## 関連

このツールは、Windows環境でUnix系コマンドを使いやすくするプロジェクトの一部です。
