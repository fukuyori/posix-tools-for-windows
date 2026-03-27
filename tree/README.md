# tree - ディレクトリツリー表示コマンド（Rust実装）

Windows 上での実用性を優先しつつ、Unix 系 `tree` に近いオプション体系で使える Rust 実装です。

## 特徴

- **Unix系互換を意識**: `tree` でよく使われるオプションを中心にサポート
- **日本語対応**: ヘルプ・エラーメッセージは日本語、ファイル名も正しく表示
- **カラー出力**: ディレクトリ、実行可能ファイル、アーカイブなどを色分け
- **Unicode対応**: 美しいツリー描画文字を使用
- **Windows隠し属性対応**: `-a` なしではドットファイルだけでなく Hidden 属性も省略
- **`.gitignore` 対応**: `--gitignore` で Git 管理向けの除外ルールを反映
- **JSON出力**: プログラムからの利用に便利
- **高速**: Rustによる効率的な実装

## インストール

### Windows (PowerShell)

```powershell
cargo build --release
Copy-Item .\target\release\tree.exe $HOME\bin\tree.exe
```

### Unix系

```bash
cargo build --release
cp target/release/tree.exe ~/.local/bin/
```

## 使用法

```
tree [オプション]... [ディレクトリ]...
```

### リスト表示オプション

| オプション | 説明 |
|-----------|------|
| `-a` | 隠しファイルも表示 |
| `-d` | ディレクトリのみ表示 |
| `-l` | シンボリックリンクをたどる |
| `-f` | フルパスを表示 |
| `-L LEVEL` | 深さをLEVELに制限 |
| `-P PATTERN` | パターンにマッチするファイルのみ |
| `-I PATTERN` | パターンにマッチするファイルを除外 |
| `--prune` | 空ディレクトリを非表示 |
| `--gitignore` | `.gitignore` のパターンを適用 |
| `--filelimit N` | N個以上のエントリは処理しない |

### ファイル情報オプション

| オプション | 説明 |
|-----------|------|
| `-s` | サイズを表示 |
| `-h` | サイズを人間可読形式で表示 |
| `-p` | パーミッションを表示 |
| `-D` | 最終更新日時を表示 |
| `-F` | 分類子を付加（/=ディレクトリ, *=実行可能） |
| `-Q` | ファイル名をクォート |

### ソートオプション

| オプション | 説明 |
|-----------|------|
| `-t` | 更新日時でソート |
| `-S` | サイズでソート |
| `-U` | ソートしない |
| `-r` | 逆順でソート |
| `--dirsfirst` | ディレクトリを先に表示 |

### 出力オプション

| オプション | 説明 |
|-----------|------|
| `-n` | カラー出力を無効化 |
| `-C` | カラー出力を有効化 |
| `-A` | ASCII文字でツリーを描画 |
| `--charset ASCII` | 文字セットをASCIIにする |
| `--charset UTF-8` | 文字セットをUnicodeにする |
| `-o FILE` | 出力をファイルに書き込む |
| `-J` | JSON形式で出力 |
| `--noreport` | 末尾のレポートを非表示 |

## 使用例

### 基本的な使用法
```bash
# カレントディレクトリを表示
tree

# 指定ディレクトリを表示
tree /path/to/directory

# 複数ディレクトリを表示
tree src tests docs
```

### 深さ制限
```bash
# 1階層のみ
tree -L 1

# 2階層まで
tree -L 2
```

### フィルタリング
```bash
# 隠しファイルも含む
tree -a

# ディレクトリのみ
tree -d

# .rsファイルのみ
tree -P "*.rs"

# node_modulesを除外
tree -I "node_modules"

# .gitignore を反映
tree --gitignore

# 複数パターンで除外
tree -I "node_modules" -I "*.log"
```

### ファイル情報の表示
```bash
# サイズを人間可読形式で
tree -sh

# パーミッション付き
tree -p

# サイズとパーミッション両方
tree -psh

# 日時も表示
tree -D
```

### ソート
```bash
# ディレクトリ優先
tree --dirsfirst

# サイズ順（大きい順）
tree -S

# 更新日時順
tree -t

# 逆順
tree -r
```

### 出力形式
```bash
# ASCII文字でツリー描画
tree -A

# 文字セットを明示
tree --charset ASCII

# カラーなし
tree -n

# JSON出力
tree -J > tree.json

# ファイルに出力
tree -o output.txt
```

### 実用例

```bash
# プロジェクト構造の確認（node_modules除外）
tree -I "node_modules|dist|.git" -L 3

# Git の除外設定を反映して確認
tree --gitignore -L 3

# ソースファイルのみ表示
tree -P "*.rs|*.py|*.js" --dirsfirst

# サイズの大きいファイルを確認
tree -sh -S

# JSON形式でドキュメント生成
tree -J --dirsfirst > structure.json
```

## `.gitignore` 互換性

`--gitignore` では、日常的に使う主要な `.gitignore` パターンをサポートしています。

- 通常のファイル・ディレクトリ除外
- `!pattern` による再許可
- 子ディレクトリごとの `.gitignore`
- `/build/` のようなルート固定パターン
- `**` を含む再帰的なパターン
- `\#` / `\!` のエスケープ
- 末尾スペースの Git 互換に近い扱い

Git 本体の `wildmatch` 完全互換までは狙っていませんが、Windows 上の実用には十分なレベルを目標にしています。

## 出力例

```
.
├── Cargo.toml
├── README.md
└── src
    ├── lib
    │   └── lib.rs
    └── main
        ├── helper.rs
        └── main.rs

3 ディレクトリ, 5 ファイル
```

### ASCII出力
```
.
|-- Cargo.toml
|-- README.md
`-- src
    |-- lib
    |   `-- lib.rs
    `-- main
        |-- helper.rs
        `-- main.rs
```

## ライセンス

MIT
