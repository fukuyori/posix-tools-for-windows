# which

Windows 向けの `which` 実装です。  
`PATH` と `PATHEXT` を使ってコマンドの実体を探しつつ、できるだけ POSIX の `which` に近い挙動になるように調整しています。

## 特徴

- `PATH` 上の実行ファイルを検索
- `PATHEXT` を使って `.exe` や `.cmd` などを自動補完
- Windows 前提でファイル名の大文字小文字を区別しない
- Windows シェルでは通常展開されないワイルドカードを内部で glob 展開
- カレントディレクトリは `PATH` に `.` が含まれている場合のみ検索
- パス区切りを含む引数は `PATH` ではなく、その指定パスだけを確認

## POSIX 寄りの方針

このツールは Windows 専用ですが、次の点で Linux / POSIX の `which` に寄せています。

- カレントディレクトリを自動では検索しない
- `foo/bar` や `C:\tools\foo` のような引数は `PATH` 検索しない
- `note*` のようなパターンを内部で展開し、Linux シェルに近い体験を提供する

一方で、Windows 向けの実用性のために次の挙動を採用しています。

- `PATHEXT` による拡張子補完を行う
- ファイル名の比較は大文字小文字を区別しない

## ビルド

```powershell
cargo build --release
```

## 使い方

```powershell
which notepad
which -a python
which cmd powershell
which note*
which .\tool
which C:\Windows\System32\where.exe
```

## 主なオプション

- `-a`, `--all`: すべての一致を表示
- `-s`, `-q`, `--silent`, `--quiet`: 出力なしで終了コードのみ返す
- `--skip-dot`: `PATH` 上の `.` を無視する
- `--show-dot`: `PATH` 上の `.` で見つかったときに `./cmd` 形式で表示する
- `--skip-tilde`: `~` で始まる `PATH` エントリを無視する
- `--show-tilde`: HOME 配下のパスを `~` 付きで表示する
- `--tty-only`: 標準出力が端末のときだけ処理する
- `-h`, `--help`: ヘルプを表示
- `-v`, `--version`: バージョンを表示

## glob 展開

PowerShell や `cmd.exe` では、Linux シェルのようにコマンド引数のワイルドカードが期待通り展開されないことがあります。  
このツールはその差を埋めるため、必要に応じて内部で glob を使って展開します。

例:

```powershell
which note*
```

この場合、`PATH` 上の実行可能ファイルを走査し、`notepad.exe` のような候補を `note*` にマッチさせます。  
パターンにパス区切りが含まれる場合は、`PATH` ではなく実際のファイルパスに対して glob を行います。

## 終了ステータス

- `0`: すべてのコマンドが見つかった
- `1`: 1 つ以上のコマンドが見つからなかった
- `2`: オプションエラー

## テスト

```powershell
cargo test
```

現在は次のような観点をテストしています。

- カレントディレクトリを `PATH` に含めない限り検索しないこと
- コマンド名 glob を大文字小文字を無視して展開すること
- 明示パスを大文字小文字を無視して解決できること
- パス付き glob を実ファイルに対して展開できること
