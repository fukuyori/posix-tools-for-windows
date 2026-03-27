# ps for Windows

Windows 上で `ps` をできるだけ POSIX / Linux 風に使えるようにするための Rust 製コマンドです。

この実装は Linux の `procfs` を再現するものではなく、Windows API から取得できるプロセス情報をもとに、`ps` に近い CLI と出力形式を提供します。

## 特徴

- POSIX 系の `ps` に近いオプション体系
- `aux` 形式の BSD スタイル表示
- `-f`, `-l`, `-o` による出力切り替え
- `--sort` と `--forest` をサポート
- Windows でも `-u/-U`, `-g/-G`, `-t` のセレクタで内部 glob を利用可能

## 対応している主なオプション

### プロセス選択

- `-e`, `-A`
- `-a`
- `-d`
- `-p`, `--pid <PID,...>`
- `-t`, `--tty <tty,...>`
- `-u`, `-U`, `--user <user,...>`
- `-g`, `-G`, `--group <group,...>`

### 出力形式

- `-f`
- `-l`
- `-o`, `--format <field[=header],...>`

使用可能な代表フィールド:

- `pid`
- `ppid`
- `pgid`
- `uid`
- `user`
- `gid`
- `group`
- `pri`
- `ni`
- `vsz`
- `rss`
- `pcpu`
- `pmem`
- `etime`
- `time`
- `tty`
- `stat`
- `comm`
- `args`
- `nlwp`
- `stime`

### GNU 拡張

- `--sort <[+|-]key>`
- `-H`, `--forest`
- `--no-headers`
- `-S`
- `--cols=N`, `--columns=N`
- `--rows=N`, `--lines=N`

### 情報表示

- `-h`, `--help`
- `-V`, `--version`

## ビルド

```powershell
cargo build --release
```

生成物:

- `target\release\ps.exe`

## 使い方

```powershell
ps
ps -ef
ps aux
ps -p 1,4,1234
ps -o pid,user,tty,time,comm
ps --sort -cpu
ps --forest
```

## Windows での内部 glob

Windows のシェルは Linux シェルのようにコマンド引数中のワイルドカードを自動展開しないことがあります。
その差を吸収するため、この `ps` では一部セレクタを内部で glob 評価します。

対象:

- `-u`, `-U`, `--user`
- `-g`, `-G`, `--group`
- `-t`, `--tty`

利用できるパターン:

- `*`
- `?`
- `[abc]`

例:

```powershell
ps -u 'NT AUTHORITY\*'
ps -g 'admin*'
ps -t 'con*'
```

挙動メモ:

- ユーザー名、グループ名、TTY の glob は大文字小文字を区別しません
- `-t` は glob を使わない場合、従来どおり部分一致でも判定します
- `-p` や `-o` など、値を 1 つの文字列として扱うオプションには内部 glob を使いません

## 実装上の注意

- Windows には Linux のようなプロセスグループや TTY の概念がそのまま存在しないため、一部の列は近似値または簡略化された値です
- `TTY` は限定的な表現です
- `nice` や一部の POSIX 意味論は Windows 上では完全再現していません
- Linux の `ps` と完全一致することを目的にはしていません

## テスト

```powershell
cargo test
```

現在は主に引数セレクタと内部 glob の挙動を確認するテストを含みます。
