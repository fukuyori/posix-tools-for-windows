# touch

Windows 向けの `touch` 実装です。Rust で書かれており、POSIX `touch` の基本挙動をできるだけ保ちつつ、Windows 上でも扱いやすいように内部で glob 展開を行います。

このツールは Windows 専用です。ファイル名の比較では Windows の実態に合わせて大文字小文字を区別しません。

## 特徴

- POSIX `touch` の基本オプションに対応
- `-d` や `--time` などの GNU 系オプションにも一部対応
- Windows 上でシェルが展開しない `*.txt` などを内部で glob 展開
- glob は大文字小文字を無視して一致
- glob が未一致でも Linux の一般的なシェル挙動に寄せて、パターン文字列をそのまま扱う
- `-r` では参照先のアクセス時刻と更新時刻を個別に引き継ぐ
- ディレクトリのタイムスタンプ更新にも対応

## 対応オプション

### POSIX 系

- `-a`
  アクセス時刻のみ変更
- `-m`
  更新時刻のみ変更
- `-c`, `--no-create`
  ファイルを新規作成しない
- `-r FILE`, `--reference=FILE`
  `FILE` の時刻を使用
- `-t STAMP`
  `[[CC]YY]MMDDhhmm[.ss]` 形式で時刻指定

### GNU 拡張

- `-d STRING`, `--date=STRING`
  日時文字列を指定
- `-h`, `--no-dereference`
  シンボリックリンク自体の時刻を変更
- `--time=WORD`
  `access`, `atime`, `use`, `modify`, `mtime` を指定可能
- `-f`
  BSD 互換のため受理するが、動作上は無視
- `--help`
- `--version`

## ビルド

```powershell
cargo build
```

リリースビルド:

```powershell
cargo build --release
```

生成物は `target\debug\touch.exe` または `target\release\touch.exe` に出力されます。

## 使い方

```text
touch [OPTION]... FILE...
```

### 基本例

現在時刻で更新:

```powershell
touch file.txt
```

存在するときだけ更新:

```powershell
touch -c file.txt
```

アクセス時刻のみ更新:

```powershell
touch -a file.txt
```

更新時刻のみ変更:

```powershell
touch -m file.txt
```

参照ファイルと同じ時刻にする:

```powershell
touch -r ref.txt target.txt
```

日時文字列を指定:

```powershell
touch -d "2024-01-15 10:30:00" file.txt
```

POSIX 形式のタイムスタンプを指定:

```powershell
touch -t 202401151030 file.txt
```

Windows でも glob を内部展開:

```powershell
touch *.txt
touch logs\*.LOG
```

## glob の扱い

Windows の `cmd.exe` や PowerShell は、Unix 系シェルのように常にワイルドカードを事前展開するわけではありません。そのためこのツールでは、引数に `*`, `?`, `[` を含む場合に内部で glob を解釈します。

動作方針:

- 一致時は該当パスへ展開
- 一致判定は大文字小文字を無視
- 未一致時は空にせず、元のパターン文字列をそのまま保持
- 不正な glob パターンでも、できるだけリテラルとして扱う

このため、Windows 上でも `touch *.txt` のような使い方をしやすくしています。

## 日時指定

### `-t` で受け付ける形式

- `MMDDhhmm`
- `YYMMDDhhmm`
- `CCYYMMDDhhmm`
- `CCYYMMDDhhmm.ss`

2 桁年は POSIX に合わせて次のように解釈します。

- `69` から `99` は `1969` から `1999`
- `00` から `68` は `2000` から `2068`

### `-d` で受け付ける例

- `2024-01-15 10:30:00`
- `2024-01-15 10:30`
- `2024-01-15`
- `2024/01/15 10:30:00`
- `now`
- `today`
- `yesterday`
- `tomorrow`
- `+1 day`
- `-2 hours`
- `+30 minutes`

## Windows 固有の注意点

- この実装は Windows API を直接使ってタイムスタンプを更新します
- ディレクトリの時刻変更もサポートします
- シンボリックリンクの扱いは Windows の権限やファイルシステムの制約を受けます
- 大文字小文字を区別しないファイルシステム前提で glob を実装しています

## テスト

```powershell
cargo test
```

現在は次のような観点を確認しています。

- case-insensitive glob 展開
- glob 未一致時のリテラル維持
- Unix epoch より前の時刻を含む FILETIME 変換

## ライセンス

MIT
