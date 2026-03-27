# du

Windows 向けの `du` 実装です。POSIX `du` をベースにしつつ、GNU `du` の実用的な拡張も取り込んでいます。Windows 上でもディレクトリ容量を Linux に近い操作感で確認できます。

## 特徴

- 既定では 1024 バイト単位でサイズを表示
- `-a` `-s` `-x` などの POSIX オプションに対応
- `-h` `--si` `-B` によるサイズ表現の切り替えをサポート
- `--max-depth` `--threshold` `--exclude` などの GNU 拡張を実装
- `--apparent-size` と `--bytes` で実サイズ基準の表示が可能
- ハードリンク重複カウント抑制やシンボリックリンク制御に対応

## 使い方

```powershell
cargo run --
cargo run -- -h
cargo run -- -sh .
cargo run -- -ah src
cargo run -- --max-depth=1 -h .
cargo run -- --exclude=target --total .
```

## 主なオプション

- `-a`
  ファイルも含めて表示します
- `-s`
  各引数の合計だけを表示します
- `-x`
  別ファイルシステムをまたがず集計します
- `-h, --human-readable`
  人間向けのサイズで表示します
- `--si`
  1000 単位の人間向けサイズで表示します
- `-B, --block-size=SIZE`
  表示ブロックサイズを指定します
- `-b, --bytes`
  バイト単位の実サイズを表示します
- `-d, --max-depth=N`
  指定深さまで表示します
- `-c, --total`
  総計を表示します
- `-S, --separate-dirs`
  サブディレクトリを含めず、ディレクトリ自体のサイズだけを扱います
- `-t, --threshold=SIZE`
  しきい値で出力を絞り込みます
- `--exclude=PATTERN`
  パターン一致を除外します
- `--time`
  更新時刻も表示します

## サイズ指定

`-B` や `--threshold` では次の単位が使えます。

- `K` `KiB`
- `M` `MiB`
- `G` `GiB`
- `KB` `MB` `GB`

## 互換性について

Windows ではファイルサイズ取得やリンク情報が Linux と完全には一致しないため、厳密な互換ではありません。それでも日常利用で必要な `du` の振る舞いにはかなり寄せています。

## テスト

```powershell
cargo test
```

## ライセンス

MIT
