# fd-rs

**fd 1.0.0 (fd互換 Rust Windows版)**

シンプルで高速、ユーザーフレンドリーな `find` の代替コマンド。
[sharkdp/fd](https://github.com/sharkdp/fd) に近い操作感を目指した Rust 実装です。特に Windows 環境で使いやすいことを重視しています。

## 特徴

- **シンプルな構文**: `fd PATTERN` で検索（`find -iname '*PATTERN*'` の代わり）
- **Windows 向け glob 対応**: `src\*.rs` のような `\` 区切りパターンも利用可能
- **スマートケース**: パターンが全て小文字なら大文字小文字を無視
- **ignore 対応**: `.gitignore` / `.fdignore` / `.ignore` を読んで除外
- **ignore の再包含に対応**: `!keep.log` のような否定パターンを評価
- **親ディレクトリ ignore を継承**: `--no-ignore-parent`、`--no-ignore-vcs` にも対応
- **正規表現 (デフォルト) と glob パターン対応**
- **色付き出力**

## 互換性

日常用途でよく使う `fd` の操作はひと通り揃っています。現時点では、手元の PowerShell ベースの CLI テストで `43/43 PASS` を確認しています。

特に次の機能は実用レベルです。

- パターン検索、スマートケース、`--glob`、`--full-path`
- `--exclude`、`-e/--extension`、`-t/--type`
- `--max-depth`、`--min-depth`、`--exact-depth`
- `--exec`、`--exec-batch`、`--format`
- `.gitignore` / `.fdignore` / `.ignore`
- ignore の否定パターン、ディレクトリ単位の ignore、親ディレクトリからの ignore 継承

一方で、本家 `fd` の完全互換ではありません。以下はまだ差分が残る可能性があります。

- gitignore の細かなエスケープ規則や特殊ケース
- パフォーマンスや並列探索の成熟度
- すべてのオプションの完全再現
- 色、TTY、シンボリックリンク、エラー表示の細部

## インストール

```bash
cargo build --release
```

生成された `target/release/fd.exe` を PATH の通った場所に配置してください。

## 使い方

```bash
fd                      # 全ファイル表示
fd pattern              # パターンにマッチするファイル
fd -e txt               # .txt ファイル
fd -t d                 # ディレクトリのみ
fd -H -I pattern        # 隠しファイル含む、ignore無視
fd -u pattern           # 上と同じ（省略形）
fd -d 2                 # 深さ2まで
fd -S +1Mi              # 1MiB以上のファイル
fd -g '*.txt'           # glob パターン
fd -g 'src\*.rs'        # Windows 風パス区切りの glob
fd -x echo {}           # 結果を echo で実行
```

## 主なオプション

| オプション | 説明 |
|-----------|------|
| `-H, --hidden` | 隠しファイル検索 |
| `-I, --no-ignore` | ignore ファイル無視 |
| `-u, --unrestricted` | `-HI` の省略形 |
| `-s, --case-sensitive` | 大文字小文字区別 |
| `-i, --ignore-case` | 大文字小文字無視 |
| `-g, --glob` | glob パターン |
| `-F, --fixed-strings` | リテラル文字列 |
| `-t, --type` | ファイルタイプ (f/d/l/x/e) |
| `-e, --extension` | 拡張子フィルタ |
| `-S, --size` | サイズフィルタ |
| `-d, --max-depth` | 最大深さ |
| `-E, --exclude` | 除外パターン |
| `-x, --exec` | コマンド実行 |
| `-X, --exec-batch` | バッチ実行 |
| `-a, --absolute-path` | 絶対パス表示 |
| `-L, --follow` | シンボリックリンクを辿る |
| `-0, --print0` | null 区切り出力 |
| `-c, --color` | 色付け (auto/always/never) |

## ignore ファイル

次の ignore ファイルを読み込みます。

- `.gitignore`
- `.fdignore`
- `.ignore`

サポートしている挙動:

- 通常の glob パターン
- `!pattern` による再包含
- `dir/` のようなディレクトリ指定
- サブディレクトリごとの ignore 継承
- 親ディレクトリの ignore 継承

関連オプション:

- `--no-ignore`
- `--no-ignore-vcs`
- `--no-ignore-parent`
- `--ignore-file <path>`

## プレースホルダ

`-x`, `-X`, `--format` で使用可能:

| プレースホルダ | 説明 |
|---------------|------|
| `{}` | フルパス |
| `{/}` | ベースネーム |
| `{//}` | 親ディレクトリ |
| `{.}` | 拡張子なしパス |
| `{/.}` | 拡張子なしベースネーム |

## ライセンス

MIT

## 謝辞

このプロジェクトは [sharkdp/fd](https://github.com/sharkdp/fd) の使い勝手と仕様を参考にしています。
