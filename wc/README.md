# wc

Windows 向けの `wc` 実装です。POSIX / GNU `wc` の主要な挙動に寄せつつ、Windows 上で扱いやすい拡張を含みます。

## 特徴

- 既定動作は `wc` と同様に `-l -w -c`
- `-c` はバイト数、`-l` は改行バイト数を生データ基準でカウント
- `-w` `-m` `-L` は文字列として処理
- `-L` はタブ幅 8 を考慮し、結合文字は幅 0 として扱う
- Windows 上でも内部 glob 展開で Linux シェルに近いファイル展開を実施
- glob は大文字小文字を区別しない
- `--files0-from` をサポート
- 独自拡張として半角文字数 / 全角文字数を表示可能

## Windows 向け glob の仕様

- `*` や `?` はディレクトリ区切りをまたがない
- ドット始まりのファイルは、パターン側でも `.` を明示した場合のみマッチ
- 展開結果は大文字小文字を無視して安定ソート
- マッチしなかったパターンはそのままファイル名として扱う

## 文字コード

既定では `UTF-8` として処理します。

文字系オプション `-w` `-m` `-L` `-H` `-F` では、`--encoding` で文字コードを切り替えられます。

- `--encoding=utf8`
- `--encoding=auto`
- `--encoding=sjis`
- `--encoding=eucjp`

`auto` を指定した場合のみ、UTF-8 を優先しつつ Shift_JIS / EUC-JP の自動判定を行います。デコードできない入力は strict にエラーとします。

## 独自拡張

- `-H`, `--halfwidth`
  半角文字数を表示します
- `-F`, `--fullwidth`
  全角文字数を表示します
- `--encoding=...`
  文字系オプションに使う文字コードを指定します

## 使い方

```powershell
cargo run -- file.txt
cargo run -- -l file.txt
cargo run -- -m --encoding=sjis file.txt
cargo run -- -L *.txt
cargo run -- --files0-from=list.bin
```

## 互換性について

この実装は Windows 専用です。POSIX / GNU `wc` の主要挙動にはかなり近づけていますが、厳密互換ではありません。

差分が残る主な点:

- 本家はシェルに任せる glob を、この実装では内部展開する
- 文字分類はロケール完全準拠ではなく、Windows 向けの実用寄り実装
- `--encoding` `-H` `-F` は独自拡張

## テスト

```powershell
cargo test
```
