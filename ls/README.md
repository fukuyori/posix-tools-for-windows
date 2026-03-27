# ls

Windows 向けの `ls` 実装です。POSIX `ls` を軸にしながら、GNU `ls` でよく使う表示オプションや色付けを取り込み、Windows 上でも Unix 系に近い一覧表示を目指しています。

## 特徴

- 通常表示、詳細表示、1 行 1 件、カンマ区切り表示に対応
- `-a` `-A` `-l` `-R` `-t` `-S` などの主要オプションをサポート
- `--color` や `--group-directories-first` など GNU 系オプションに対応
- `-h` `--si` による人間向けサイズ表示が可能
- Windows 上でも内部 glob 展開で `*.txt` などを扱える
- 詳細表示では時刻形式や属性に応じた出力を行う

## 使い方

```powershell
cargo run --
cargo run -- -la
cargo run -- -lh
cargo run -- -lt
cargo run -- -R src
cargo run -- --color=always *.rs
```

## 主なオプション

- `-a, --all`
  隠しエントリも表示します
- `-A, --almost-all`
  `.` と `..` を除いて表示します
- `-d, --directory`
  ディレクトリの中身ではなくディレクトリ自体を表示します
- `-F, --classify`
  型識別子を付けます
- `-i, --inode`
  inode 番号を表示します
- `-l`
  詳細形式で表示します
- `-R, --recursive`
  再帰的に表示します
- `-r`
  ソート順を反転します
- `-S`
  サイズ順でソートします
- `-t`
  時刻順でソートします
- `-1`
  1 行 1 件で表示します
- `-h, --human-readable`
  サイズを K/M/G 表記にします
- `--si`
  1000 単位のサイズ表記にします
- `--color=WHEN`
  `auto` `always` `never` を指定できます
- `--group-directories-first`
  ディレクトリを先に並べます
- `--time-style=STYLE`
  時刻表示形式を切り替えます

## 互換性について

この実装は Windows 向けです。シンボリックリンクや所有者情報など、Windows API から取れる情報に基づいて表示します。glob はシェルではなくコマンド内部で展開します。

## テスト

```powershell
cargo test
```

## ライセンス

MIT
