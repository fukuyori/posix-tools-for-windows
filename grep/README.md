# grep

Windows 上で使いやすいことを重視しつつ、できるだけ POSIX/GNU `grep` に近い操作感を目指した Rust 製の `grep` 実装です。

UTF-8 / Shift_JIS / EUC-JP の自動判定に対応し、Windows シェルが展開しない glob を内部で展開します。これにより、Windows でも Linux に近い感覚で `*.txt` や `src/*.rs` を指定できます。

## 特徴

- POSIX / GNU `grep` の主要オプションをほぼ網羅
- デフォルトは POSIX どおり基本正規表現（BRE）。`-E` で ERE、`-F` で固定文字列
- Windows でも glob を内部展開
- glob の大文字小文字は POSIX 寄りに常に区別
- Windows の `\` 区切りを内部で `/` と同様に扱う
- CRLF 改行を自動処理（`-x` や `$` が期待どおり動作）
- `--include` / `--exclude` / `--exclude-from` / `--exclude-dir` に対応
- UTF-8 / Shift_JIS / EUC-JP の自動判定

## ビルド

```powershell
cargo build --release
```

生成物は `target/release/grep.exe` です。

## 使い方

```text
grep [オプション] パターン [ファイル...]
grep [オプション] -e パターン [-e パターン...] [ファイル...]
grep [オプション] -f ファイル [ファイル...]
```

例:

```powershell
grep hello *.txt
grep -n TODO src\*.rs
grep -r --include=*.rs fn src
grep -i error logs\*.log
```

## Windows での glob 方針

Windows の `cmd.exe` や PowerShell は、Linux のシェルのように `*.txt` を自動展開しません。このツールはその差分を埋めるため、ファイル引数の glob を内部で展開します。

現在の方針は次の通りです。

- `*.txt` や `src/*.rs` のようなパターンを内部で展開する
- Windows でも glob の大文字小文字は自動で無視しない
- Windows の `src\*.rs` も内部で正規化して扱う
- マッチしなかった glob はそのままファイル名として処理する

そのため、Windows でも Linux に近い感覚でパターンを書けますが、ケース感度まで含めて POSIX 寄りになる点には注意してください。

## 主なオプション

- `-E`, `--extended-regexp`: 拡張正規表現（ERE）
- `-F`, `--fixed-strings`: 固定文字列検索
- `-G`, `--basic-regexp`: 基本正規表現（BRE、デフォルト）
- `-i`, `--ignore-case` / `--no-ignore-case`: 大文字小文字の扱い
- `-w`, `-x`: 単語単位 / 行全体マッチ
- `-v`, `--invert-match`: マッチしない行を表示
- `-n`, `-b`: 行番号 / バイトオフセットを表示
- `-c`, `-l`, `-L`, `-o`, `-m NUM`: 出力制御
- `-q`, `--quiet` / `-s`, `--no-messages`: 出力抑制 / エラーメッセージ抑制
- `-A`, `-B`, `-C`, `-NUM`: コンテキスト表示（`--group-separator` / `--no-group-separator` 対応）
- `-r`, `--recursive`: 再帰検索（ファイル省略時は `.`、走査中のシンボリックリンクは辿らない）
- `-R`, `--dereference-recursive`: 再帰検索でシンボリックリンクを辿る
- `-d`, `--directories=ACTION`: ディレクトリの扱い（read / skip / recurse）
- `--include=GLOB` / `--exclude=GLOB` / `--exclude-from=FILE` / `--exclude-dir=GLOB`
- `-a`, `-I`, `--binary-files=TYPE`: バイナリファイルの扱い
- `-z`, `--null-data` / `-Z`, `--null`: NUL 区切り入出力
- `--color[=WHEN]`: 色付き出力
- `--label=LABEL`, `-T`, `--line-buffered`（互換受理）ほか

詳細は次で確認できます。

```powershell
grep --help
```

## GNU grep 互換の挙動

- 終了コード: 0 = マッチあり、1 = なし、2 = エラー。`-q` はマッチがあればエラーが起きても 0
- `-L` はファイルがリストされたときに 0（GNU grep 3.2 以降の仕様）
- `-e` / 位置引数パターン内の改行はパターン区切り（OR）
- `-f` のパターンファイルでは空行は「全行にマッチ」、空ファイルは「何にもマッチしない」
- BRE では `+ ? | ( ) { }` はリテラル、`\+ \? \| \( \) \{ \}` がメタ文字（`\<` `\>` = 単語境界）
- 旧式の同義語 `-y`（= `-i`）や `-U` `-u` `--mmap` などは互換のため受理

## テスト

```powershell
cargo test
```

glob 展開、Windows 区切り文字の正規化、BRE/ERE の解釈、CRLF 処理、行分割とオフセット計算などをテストしています。
