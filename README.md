# posix-tools-for-windows

Windows 向けに Rust で実装した POSIX / GNU 系コマンド集です。PowerShell や `cmd.exe` でも Unix 系に近い操作感を得られるように、各ツールが内部で glob 展開や Windows 向けの挙動調整を行います。

リポジトリ:

`https://github.com/fukuyori/posix-tools-for-windows.git`

## 収録ツール

- `awk`
- `cat`
- `comm`
- `cp`
- `cut`
- `df`
- `diff`
- `du`
- `fd`
- `find`
- `fold`
- `grep`
- `groff`
- `head`
- `join`
- `kill`
- `less`
- `ls`
- `mv`
- `paste`
- `patch`
- `ps`
- `pwd`
- `rm`
- `sed`
- `sort`
- `split`
- `tail`
- `tee`
- `top`
- `touch`
- `tr`
- `tree`
- `uniq`
- `wc`
- `which`
- `xargs`

## 方針

- Windows 上でも POSIX / GNU の主要な使い勝手を再現する
- 実用性を優先しつつ、各ツールごとに README で差分を明示する
- シェル未展開の glob をコマンド側で補う
- 日本語環境や Windows 固有のパス表現を考慮する

## ディレクトリ構成

各ツールは独立した Cargo crate として配置しています。

- [`awk`](./awk/)
- [`cat`](./cat/)
- [`comm`](./comm/)
- [`cp`](./cp/)
- [`cut`](./cut/)
- [`df`](./df/)
- [`diff`](./diff/)
- [`du`](./du/)
- [`fd`](./fd/)
- [`find`](./find/)
- [`fold`](./fold/)
- [`grep`](./grep/)
- [`groff`](./groff/)
- [`head`](./head/)
- [`join`](./join/)
- [`kill`](./kill/)
- [`less`](./less/)
- [`ls`](./ls/)
- [`mv`](./mv/)
- [`paste`](./paste/)
- [`patch`](./patch/)
- [`ps`](./ps/)
- [`pwd`](./pwd/)
- [`rm`](./rm/)
- [`sed`](./sed/)
- [`sort`](./sort/)
- [`split`](./split/)
- [`tail`](./tail/)
- [`tee`](./tee/)
- [`top`](./top/)
- [`touch`](./touch/)
- [`tr`](./tr/)
- [`tree`](./tree/)
- [`uniq`](./uniq/)
- [`wc`](./wc/)
- [`which`](./which/)
- [`xargs`](./xargs/)

## 使い方

各ディレクトリで個別にビルド・実行します。

```powershell
cd ls
cargo run -- -la
```

テストも各 crate ごとに実行できます。

```powershell
cd grep
cargo test
```

## ライセンス

MIT
