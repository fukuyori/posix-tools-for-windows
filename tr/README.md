# tr - POSIX準拠文字変換コマンド（Rust実装）

Windows環境向けのPOSIX準拠`tr`コマンドのRust実装です。GNU拡張も含みます。

## 特徴

- **POSIX準拠**: 標準的な`tr`コマンドの動作を再現
- **GNU拡張対応**: `-t`（切り詰め）、`[char*]`（繰り返し）など
- **バイト列処理**: 入力を文字列へ変換せず 8bit データとして処理
- **文字クラス対応**: `[:alnum:]`, `[:alpha:]`, `[:digit:]`など全POSIX文字クラス
- **高速**: Rustによる効率的な実装

## インストール

```bash
cargo build --release
cp target/release/tr.exe ~/.local/bin/  # または任意のPATHディレクトリ
```

## 使用法

```
tr [オプション]... SET1 [SET2]
```

標準入力から読み込み、バイト単位で文字の変換・削除を行い、標準出力に書き出します。

### オプション

| オプション | 説明 |
|-----------|------|
| `-c, -C, --complement` | SET1の補集合を使用 |
| `-d, --delete` | SET1の文字を削除（変換しない） |
| `-s, --squeeze-repeats` | 連続する同一文字を1つに圧縮 |
| `-t, --truncate-set1` | SET1をSET2の長さに切り詰めてから変換 |

### SET の指定方法

| 指定方法 | 説明 |
|---------|------|
| `abc` | リテラル文字列（UTF-8 引数はそのバイト列として扱う） |
| `a-z` | 文字範囲（aからzまで） |
| `[:alnum:]` | 英数字 |
| `[:alpha:]` | 英字 |
| `[:blank:]` | 水平空白（スペースとタブ） |
| `[:cntrl:]` | 制御文字 |
| `[:digit:]` | 数字 |
| `[:graph:]` | 印字可能文字（空白除く） |
| `[:lower:]` | 小文字 |
| `[:print:]` | 印字可能文字（空白含む） |
| `[:punct:]` | 句読点 |
| `[:space:]` | 空白文字 |
| `[:upper:]` | 大文字 |
| `[:xdigit:]` | 16進数字 |
| `[char*]` | SET1の長さまで繰り返し |
| `[char*N]` | N回繰り返し |
| `\n` | 改行 |
| `\t` | タブ |
| `\r` | 復帰 |
| `\NNN` | 8進数で文字指定 |

## 使用例

### 大文字・小文字変換
```bash
# 小文字を大文字に
echo "hello world" | tr 'a-z' 'A-Z'
# HELLO WORLD

# 文字クラスを使用
echo "hello world" | tr '[:lower:]' '[:upper:]'
# HELLO WORLD
```

### 文字の削除
```bash
# 数字を削除
echo "abc123def" | tr -d '[:digit:]'
# abcdef

# Windows改行をUnix改行に（CRを削除）
tr -d '\r' < windows.txt > unix.txt
```

### 文字の圧縮
```bash
# 連続するスペースを1つに
echo "hello   world" | tr -s ' '
# hello world

# 連続する改行を1つに
tr -s '\n' < file.txt
```

### 補集合
```bash
# 英数字以外をすべてXに
echo "hello, world!" | tr -c '[:alnum:]' 'X'
# helloXXworldX

# 単語ごとに改行（英数字以外を改行に変換・圧縮）
echo "hello, world! 123" | tr -cs '[:alnum:]' '\n'
# hello
# world
# 123
```

### 複数文字の変換
```bash
# ROT13暗号
echo "hello" | tr 'a-zA-Z' 'n-za-mN-ZA-M'
# uryyb
```

### 切り詰め（-t）
```bash
# SET2が短い場合、通常は最後の文字で埋める
echo "abcdef" | tr 'abcdef' '123'
# 123333

# -t で SET1 を切り詰め
echo "abcdef" | tr -t 'abcdef' '123'
# 123def
```

### ファイルを処理
```bash
# リダイレクト
tr 'a-z' 'A-Z' < input.txt > output.txt

# パイプ
Get-Content input.txt -Raw | tr 'a-z' 'A-Z'
```

## 注意事項

- `tr`は標準入力のみを処理します。ファイル名引数や内部 glob 展開は扱いません
- SET2がSET1より短い場合、SET2の最後の文字が繰り返されます（`-t`で変更可能）
- `-c`と`-d`を組み合わせると、SET1に含まれない文字をすべて削除できます
- POSIX に寄せるため、文字エンコーディングの自動判定は行わず、入力はそのまま 8bit データとして扱います

## ライセンス

MIT

## 関連

このツールは、Windows環境でUnix系コマンドを使いやすくするプロジェクトの一部です。
`sort`、`uniq`と組み合わせて使用することで、強力なテキスト処理が可能になります。

```bash
# 単語の出現頻度を数える
cat file.txt | tr -cs '[:alnum:]' '\n' | sort | uniq -c | sort -rn
```
