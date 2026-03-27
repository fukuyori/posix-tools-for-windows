# split - POSIX準拠ファイル分割コマンド（Rust実装）

Windows環境向けのPOSIX準拠`split`コマンドのRust実装です。GNU拡張も含みます。

## 特徴

- **POSIX準拠**: 標準的な`split`コマンドの動作を再現
- **GNU拡張対応**: バイト分割、N個分割、数字/16進サフィックスなど
- **Windows向けglob補完**: shell が展開しない環境でも内部で pathname expansion を再現
- **日本語対応**: ヘルプ・エラーメッセージは日本語
- **柔軟な分割**: 行数、バイト数、ファイル数での分割に対応
- **高速**: Rustによる効率的な実装

## インストール

```bash
cargo build --release
cp target/release/split.exe ~/.local/bin/  # または任意のPATHディレクトリ
```

## 使用法

```
split [オプション]... [ファイル [プレフィックス]]
```

出力ファイル名は「プレフィックス + サフィックス」の形式です。
デフォルトのプレフィックスは `x`、サフィックスは `aa`, `ab`, ... です。

### 分割モードオプション

| オプション | 説明 |
|-----------|------|
| `-l, --lines=N` | N行ごとに分割（デフォルト：1000） |
| `-b, --bytes=SIZE` | SIZEバイトごとに分割 |
| `-C, --line-bytes=SIZE` | 最大SIZEバイトで行を保持して分割 |
| `-n, --number=N` | N個のファイルに均等分割 |
| `-n l/N` | 行ベースでN個に分割 |

### サフィックスオプション

| オプション | 説明 |
|-----------|------|
| `-a, --suffix-length=N` | サフィックス長（デフォルト：2） |
| `-d` | 数字サフィックス（00, 01, ...） |
| `-x, --hex-suffix` | 16進サフィックス（00, 01, ... 0f, 10, ...） |
| `--additional-suffix=SUF` | 追加サフィックス（例：.txt） |
| `--numeric-suffix=FROM` | 開始番号を指定 |

### その他のオプション

| オプション | 説明 |
|-----------|------|
| `-e, --elide-empty-files` | 空ファイルを作成しない |
| `--verbose` | 作成ファイル名を表示 |
| `--filter=CMD` | 出力をコマンドに渡す |

### サイズ指定

数字の後にサフィックスを付けることができます：
- `b` = 512バイト
- `K` = 1024バイト (KiB)
- `M` = 1048576バイト (MiB)
- `G` = 1073741824バイト (GiB)
- `KB` = 1000バイト
- `MB` = 1000000バイト
- `GB` = 1000000000バイト

## 使用例

### 行数で分割
```bash
# デフォルト（1000行ごと）
split file.txt

# 100行ごと
split -l 100 file.txt

# カスタムプレフィックス
split -l 100 file.txt part_
# → part_aa, part_ab, part_ac, ...
```

### バイト数で分割
```bash
# 1MBごと
split -b 1M file.bin

# 10KBごと、数字サフィックス
split -b 10K -d file.bin chunk
# → chunk00, chunk01, chunk02, ...
```

### N個のファイルに分割
```bash
# 5個のファイルに均等分割（バイト単位）
split -n 5 file.bin

# 5個のファイルに行単位で分割
split -n l/5 file.txt
```

### 行を保持してバイト分割
```bash
# 最大1KBで、行の途中で切らない
split -C 1K file.txt
```

### カスタムサフィックス
```bash
# 数字サフィックス（3桁）
split -d -a 3 file.txt part_
# → part_000, part_001, part_002, ...

# 16進サフィックス
split -x file.txt
# → x00, x01, ... x0f, x10, ...

# 追加サフィックス（拡張子）
split -d --additional-suffix=.txt file.txt chunk
# → chunk00.txt, chunk01.txt, ...
```

### 詳細表示
```bash
# 作成ファイル名を表示
split -l 100 --verbose file.txt
# creating file 'xaa'
# creating file 'xab'
# ...
```

### 標準入力から
```bash
# コマンドの出力を分割
cat largefile.txt | split -l 1000
```

### Windows上でのglob展開
```bash
# PowerShell / cmd.exe でも内部で shell 風に展開
split *.txt
```

- `*`, `?`, `[]` を含む位置引数は内部で glob 展開されます
- マッチしない場合は POSIX shell 同様、パターン文字列をそのまま使います
- 複数マッチした場合は Linux の shell 展開後と同じように位置引数として解釈されます
- Windows 専用実装のため、ファイル名の大文字小文字は区別しません
- 先頭が `.` の名前は、パターン側も `.` で始めたときだけマッチします

## 分割されたファイルの結合

```bash
# アルファベットサフィックスの場合
cat xaa xab xac > combined.txt

# 数字サフィックスの場合
cat x00 x01 x02 > combined.txt

# ワイルドカード使用（順序に注意）
cat x* > combined.txt   # アルファベット順でOK
```

## 注意事項

- `-b`（バイト分割）は行の途中で切れる可能性があります
- `-C`（行バイト分割）は行を保持しますが、1行が最大サイズを超える場合はそのまま出力されます
- `-n`（N個分割）はバイト単位の均等分割です
- `-n l/N`を使用すると行単位でN個に分割されます

## ライセンス

MIT

## 関連

このツールは、Windows環境でUnix系コマンドを使いやすくするプロジェクトの一部です。
大きなログファイルの分割、バックアップの分割などに活用できます。

```bash
# 使用例：大きなログファイルを日付ごとに処理しやすいサイズに分割
split -l 10000 -d --additional-suffix=.log access.log access_
```
