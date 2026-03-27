# fold (Rust)

POSIX `fold` 互換のテキスト折り返しツール（Windows対応版）。

## 概要

- 標準入力またはファイルを指定して、指定幅で行を折り返す。
- `-b`, `-s`, `-w` (と `-WIDTH`) などの POSIX/GNU オプションに対応。
- UTF-8/Shift_JIS/EUC-JP/ISO-2022-JP の自動判定を持つ。
- `glob` クレートでファイル名ワイルドカード展開を実施。
- Windows での `foo/bar/*.txt` のような `/` パスも補完して動くよう改良済み。

## ビルド

```sh
cargo build --release
```

## 実行

```sh
./target/release/fold [オプション]... [ファイル]...
```

### 例

- `fold file.txt`  (80 文字幅)
- `fold -w 40 file.txt`
- `fold -60 file.txt` (`-w 60` と同じ)
- `fold -s file.txt`
- `fold -b file.txt`
- `fold *.txt` (glob 展開)
- `cat file | fold -w 60`

## オプション

- `-b`, `--bytes` : バイト単位で折り返し
- `-s`, `--spaces` : 空白単位で折り返し（語を分割しない）
- `-w N`, `--width=N` : 折り返し幅を指定
- `-N` : `-w N` と同等
- `--help`, `--version`

## Windows固有動作

- glob 展開時、入力パターンに `/` が含まれる場合、`\\` に置換しても試行する。  
- ファイル名の大文字小文字は OS のファイルシステムに依存する。

## テスト

```sh
cargo test
```
