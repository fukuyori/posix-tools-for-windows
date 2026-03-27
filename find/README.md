# find

Rust で実装した `find` コマンドです。POSIX/GNU `find` のよく使う挙動をベースにしつつ、Windows 上でも Linux に近い感覚で使えることを重視しています。

特に Windows では、シェルが展開しない開始パス glob を `find` 自身が展開し、`-path` や `-name` でも `/` と `\` の違いを吸収して POSIX 寄りに扱います。

## 特徴

- POSIX でよく使う `find` の書式をそのまま使いやすい
- `-iname`, `-regex`, `-maxdepth`, `-mindepth`, `-empty`, `-delete` などの GNU 系オプションに対応
- Linux/macOS/Windows で動作
- Windows でも開始パス glob を内部展開
- Windows でも glob パターンを POSIX 風に扱う
- `\(` `\)` `\!` `\;` と `(` `)` `!` `;` の両方を受理しやすい
- ヘルプとエラーメッセージは日本語

## Windows での互換方針

Windows 版では、Linux の `find` をそのまま移植したときに引っかかりやすい点を吸収します。

- 開始パスの glob を `find` 自身が展開します
- glob パターン中の `\` は `/` と同等に扱います
- `-path` / `-ipath` の比較時も内部で `/` 区切りに正規化します

そのため、Windows 上でも次のような書き方がほぼ同じ意味で動きます。

```powershell
.\find.exe .\src\* -name "*.rs"
.\find.exe ./src/* -name "*.rs"
.\find.exe . -path "*/src/*.rs"
.\find.exe . -path "*\src\*.rs"
```

## ビルド

```bash
cargo build --release
```

生成物は `target/release/find.exe` です。

## 使い方

```text
find [パス...] [式]
```

パスを省略すると `.` を開始点にします。

## 代表的な使用例

```bash
# カレントディレクトリ以下を列挙
find .

# Rust ファイルを検索
find . -name "*.rs"

# 大文字小文字を無視してログを検索
find . -iname "*.log"

# 1 MiB より大きい通常ファイルを検索
find . -type f -size +1M

# 2 階層までに制限
find . -maxdepth 2 -type f

# 空ディレクトリを検索
find . -type d -empty

# OR 条件
find . \( -name "*.c" -o -name "*.h" \)

# printf 形式で出力
find . -type f -printf "%p %s bytes\n"
```

## Windows での使用例

### PowerShell

PowerShell では括弧や `!` が解釈に影響することがあるため、引用するか `\(` のように書くと安全です。

```powershell
.\find.exe . "(" -name "*.txt" -o -name "*.rs" ")"
.\find.exe . "\(" -name "*.txt" -o -name "*.rs" "\)"
.\find.exe . "\!" -name "*.log"
```

開始パス glob は `find` 側で展開されます。

```powershell
.\find.exe .\src\* -type f
.\find.exe ./src/* -type f
```

`-path` も POSIX 風パターンで使えます。

```powershell
.\find.exe . -path "*/src/*.rs"
```

### cmd.exe

```cmd
find.exe . -type f -name *.txt
find.exe . -exec cmd /c type {} ;
find.exe . \( -name *.c -o -name *.h \)
```

### Git Bash / MSYS2 / WSL

Unix 系シェルでは通常の `find` と同じ感覚で使えます。

```bash
./find.exe . \( -name "*.c" -o -name "*.h" \)
./find.exe . -type f -exec cat {} \;
./find.exe . -path "*/src/*.rs"
```

## 対応している主な述語・アクション

- 名前系: `-name`, `-iname`, `-path`, `-ipath`, `-regex`, `-iregex`
- 種別系: `-type`, `-xtype`
- サイズ・時刻系: `-size`, `-atime`, `-ctime`, `-mtime`, `-amin`, `-cmin`, `-mmin`, `-newer`, `-newerXY`
- 所有者・権限系: `-user`, `-group`, `-uid`, `-gid`, `-nouser`, `-nogroup`, `-perm`, `-readable`, `-writable`, `-executable`
- その他: `-empty`, `-links`, `-inum`, `-samefile`, `-true`, `-false`
- アクション: `-print`, `-printf`, `-ls`, `-exec`, `-execdir`, `-delete`, `-prune`, `-quit`

## 注意点

- これは Windows 標準の `find.exe` ではなく、POSIX 風の別実装です
- Windows のパスを比較する内部処理では `/` 区切りに正規化しています
- すべての GNU `find` 拡張を完全再現することは目的にしていません

## テスト

```bash
cargo test
```

Windows では、開始パス glob 展開と POSIX 風区切りでの `-path` マッチもテストしています。

## ライセンス

MIT
