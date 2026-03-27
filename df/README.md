# df

Windows 向けの `df` 実装です。POSIX `df` の基本出力に加えて、GNU `df` でよく使う表示オプションを扱えるようにしています。

## 特徴

- 既定では 1024 バイト単位でディスク使用量を表示
- `-P` による POSIX 形式、`-k` による 1024 バイト単位表示に対応
- `-h` `-H` `-B` でサイズ表示形式を切り替え可能
- `-T` `-t` `-x` によるファイルシステム種別表示・絞り込みに対応
- `-i` による inode 表示をサポート
- Windows 上でも引数パスの glob 展開に対応

## 使い方

```powershell
cargo run --
cargo run -- -h
cargo run -- -hT
cargo run -- --total
cargo run -- C:\Windows\System32\*.dll
```

## 主なオプション

- `-P`
  POSIX 出力形式で表示します
- `-k`
  1024 バイトブロック単位で表示します
- `-h, --human-readable`
  1024 単位の人間向けサイズで表示します
- `-H, --si`
  1000 単位の人間向けサイズで表示します
- `-B, --block-size=SIZE`
  任意のブロックサイズを指定します
- `-i, --inodes`
  inode 使用状況を表示します
- `-l, --local`
  ローカルファイルシステムだけを表示します
- `-T, --print-type`
  ファイルシステムタイプ列を表示します
- `-t, --type=TYPE`
  指定タイプだけを表示します
- `-x, --exclude-type=TYPE`
  指定タイプを除外します
- `--total`
  合計行を表示します

## サイズ指定

`-B` では次のような単位を使えます。

- `K` `M` `G` `T`
- `KB` `MB` `GB` `TB`

## 互換性について

この実装は Windows 向けです。Linux の `df` と同じ CLI を意識していますが、取得できるファイルシステム情報は Windows API に依存します。

## テスト

```powershell
cargo test
```

## ライセンス

MIT
