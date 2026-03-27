# grep

Windows 上で使いやすいことを重視しつつ、できるだけ POSIX/GNU `grep` に近い操作感を目指した Rust 製の `grep` 実装です。

UTF-8 / Shift_JIS / EUC-JP の自動判定に対応し、Windows シェルが展開しない glob を内部で展開します。これにより、Windows でも Linux に近い感覚で `*.txt` や `src/*.rs` を指定できます。

## 特徴

- POSIX 系 `grep` に近い主要オプションを実装
- Windows でも glob を内部展開
- glob の大文字小文字は POSIX 寄りに常に区別
- Windows の `\` 区切りを内部で `/` と同様に扱う
- `--include` / `--exclude` / `--exclude-dir` に対応
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

- `-E`, `--extended-regexp`: 拡張正規表現
- `-F`, `--fixed-strings`: 固定文字列検索
- `-G`, `--basic-regexp`: 基本正規表現
- `-i`, `--ignore-case`: 大文字小文字を無視
- `-n`, `--line-number`: 行番号を表示
- `-r`, `--recursive`: 再帰検索
- `-c`, `--count`: マッチ数のみ表示
- `-l`, `--files-with-matches`: マッチしたファイル名のみ表示
- `-L`, `--files-without-match`: マッチしないファイル名のみ表示
- `--include=GLOB`: 対象ファイルを絞り込み
- `--exclude=GLOB`: 対象ファイルを除外
- `--exclude-dir=GLOB`: 対象ディレクトリを除外
- `--color[=WHEN]`: 色付き出力

詳細は次で確認できます。

```powershell
grep --help
```

## テスト

```powershell
cargo test
```

glob 展開、Windows 区切り文字の正規化、パス付き `--include` / `--exclude` 相当の挙動をテストしています。
