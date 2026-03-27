# tail (Rust版)

POSIX.1-2017準拠 + GNU拡張 + 日本語エンコーディング対応 + ハイライト機能を備えた `tail` コマンドのRust実装です。

## 特徴

- **POSIX.1-2017準拠**: 標準的な `tail` の動作を完全サポート
- **GNU拡張対応**: `-F`, `--retry`, `--pid` などの拡張オプション
- **マルチエンコーディング**: UTF-8, UTF-16LE/BE, Shift_JIS, EUC-JP を自動判定
- **ハイライト機能**: POSIX ERE互換の正規表現ベースの色付け
- **Windows対応**: PowerShell / Windows Terminal での日本語表示に対応

## インストール

```bash
cargo build --release
```

バイナリは `target/release/tail` に生成されます。

## 基本的な使い方

```bash
# 末尾10行を表示
tail file.txt

# 末尾20行を表示
tail -n 20 file.txt

# ファイルを追跡（Ctrl+Cで終了）
tail -f file.log

# ハイライト付きで追跡
tail -f --highlight=highlight.toml app.log
```

## オプション一覧

### POSIX標準オプション

| オプション | 説明 |
|-----------|------|
| `-c, --bytes=[+]NUM` | 末尾NUMバイトを出力（`+`で先頭からNUM以降） |
| `-f, --follow` | ファイルへの追記を追跡 |
| `-n, --lines=[+]NUM` | 末尾NUM行を出力（デフォルト: 10） |

### GNU拡張オプション

| オプション | 説明 |
|-----------|------|
| `-F` | `--follow=name --retry` と同等 |
| `--follow[=HOW]` | 追跡モード（`descriptor` または `name`） |
| `--retry` | ファイルがアクセス不能でも再試行 |
| `-s, --sleep-interval=N` | 追跡間隔をN秒に設定（デフォルト: 1.0） |
| `--pid=PID` | プロセスPID終了後に終了 |
| `-q, --quiet` | ヘッダーを非表示 |
| `-v, --verbose` | 常にヘッダーを表示 |

### ハイライトオプション

| オプション | 説明 |
|-----------|------|
| `--highlight=FILE` | TOML設定ファイルでハイライトを有効化 |

## ハイライト機能

### 設定ファイルの読み込み

1. `--highlight=FILE` で明示的に指定
2. カレントディレクトリの `tail-highlight.toml` を自動読み込み
3. `~/.config/tail/highlight.toml` を自動読み込み

### 設定ファイルの書式 (TOML)

```toml
[[rule]]
pattern = '正規表現'    # POSIX ERE 互換（リテラル文字列推奨）
fg = "色指定"           # 前景色（省略可）
bg = "色指定"           # 背景色（省略可）
bold = true             # 太字（省略可）
underline = true        # 下線（省略可）
```

### 正規表現の書式 (POSIX ERE 互換)

`grep -E`、`sed -E`、`awk` と同じ書き方が使えます。

#### 基本

| パターン | 説明 | 例 |
|---------|------|-----|
| `.` | 任意の1文字 | `a.c` → abc, aXc |
| `*` | 0回以上の繰り返し | `ab*c` → ac, abc, abbc |
| `+` | 1回以上の繰り返し | `ab+c` → abc, abbc |
| `?` | 0回または1回 | `ab?c` → ac, abc |
| `^` | 行頭 | `^ERROR` |
| `$` | 行末 | `完了$` |

#### 文字クラス

| パターン | 説明 | 例 |
|---------|------|-----|
| `[abc]` | a, b, c のいずれか | `[Ee]rror` |
| `[a-z]` | a から z の範囲 | `[A-Za-z]+` |
| `[0-9]` | 数字 | `[0-9]{4}` |
| `[^abc]` | a, b, c 以外 | `[^0-9]` |

#### グループと選択

| パターン | 説明 | 例 |
|---------|------|-----|
| `(abc)` | グループ化 | `(ab)+` |
| `a\|b` | OR（a または b） | `ERROR\|FATAL` |

#### 繰り返し回数

| パターン | 説明 | 例 |
|---------|------|-----|
| `{n}` | ちょうどn回 | `[0-9]{4}` → 4桁の数字 |
| `{n,}` | n回以上 | `[0-9]{2,}` |
| `{n,m}` | n回以上m回以下 | `[0-9]{2,4}` |

#### 特殊文字のエスケープ

以下の文字はバックスラッシュでエスケープします：

```
.  *  +  ?  ^  $  [  ]  (  )  {  }  |  \
```

例：
```toml
pattern = '\[ERROR\]'     # [ERROR] にマッチ
pattern = '192\.168\.'    # 192.168. にマッチ
```

#### 拡張機能

| パターン | 説明 | 例 |
|---------|------|-----|
| `(?i)` | 大文字小文字を区別しない | `(?i)error` |

### 色の指定方法

#### 1. 名前指定（8色）

```toml
fg = "red"
bg = "black"
```

対応色: `black`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`, `white`

#### 2. 256色インデックス指定

```toml
fg = { index = 196 }
```

#### 3. TrueColor (RGB)

```toml
fg = { rgb = [255, 80, 80] }
```

### 設定例

```toml
# tail-highlight.toml

# エラー（赤背景白文字）
[[rule]]
pattern = '(ERROR|FATAL|FAILED)'
fg = "white"
bg = "red"
bold = true

# 警告（黄色）
[[rule]]
pattern = '(WARN|WARNING)'
fg = "yellow"
bold = true

# 日付 (YYYY-MM-DD)
[[rule]]
pattern = '[0-9]{4}-[0-9]{2}-[0-9]{2}'
fg = "cyan"

# 時刻 (HH:MM:SS)
[[rule]]
pattern = '[0-9]{2}:[0-9]{2}:[0-9]{2}'
fg = "cyan"

# IPアドレス
[[rule]]
pattern = '[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}'
fg = "magenta"

# URL
[[rule]]
pattern = 'https?://[^ ]*'
fg = "blue"
underline = true

# 大文字小文字を区別しない
[[rule]]
pattern = '(?i)exception'
fg = "red"
```

## 対応エンコーディング

| エンコーディング | 判定方法 |
|-----------------|---------|
| UTF-8 | BOM (EF BB BF) または内容解析 |
| UTF-16LE | BOM (FF FE) |
| UTF-16BE | BOM (FE FF) |
| Shift_JIS | 内容解析 |
| EUC-JP | 内容解析 |

## 依存クレート

| クレート | 用途 |
|---------|------|
| `encoding_rs` | 文字エンコーディング変換 |
| `glob` | ワイルドカード展開 |
| `regex` | 正規表現 |
| `toml` | 設定ファイル解析 |
| `serde` | シリアライズ |
| `dirs` | 設定ディレクトリ取得 |

## ライセンス

MIT
