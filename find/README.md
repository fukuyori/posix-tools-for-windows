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

- 名前系: `-name`, `-iname`, `-path`, `-ipath`, `-lname`, `-ilname`, `-regex`, `-iregex`, `-regextype`
- 種別系: `-type`, `-xtype`（`f,d` のようなカンマ区切り OR にも対応）, `-fstype`
- サイズ・時刻系: `-size`, `-atime`, `-ctime`, `-mtime`, `-amin`, `-cmin`, `-mmin`, `-used`,
  `-newer`, `-anewer`, `-cnewer`, `-newerXY`（`B`=作成時刻, `t`=絶対時刻・`@epoch` も可）, `-daystart`
- 所有者・権限系: `-user`, `-group`, `-uid`, `-gid`, `-nouser`, `-nogroup`, `-perm`, `-readable`, `-writable`, `-executable`
- その他: `-empty`, `-links`, `-inum`, `-samefile`, `-true`, `-false`
- アクション: `-print`, `-print0`, `-fprint`, `-fprint0`, `-printf`, `-fprintf`, `-ls`, `-fls`,
  `-exec`, `-execdir`, `-ok`, `-okdir`, `-delete`, `-prune`, `-quit`
- グローバル: `-H`, `-L`, `-P`, `-follow`, `-maxdepth`, `-mindepth`, `-depth`, `-xdev`/`-mount`

## GNU find 互換の挙動

- 走査はプレオーダー深さ優先（`-depth` でポストオーダー）で、GNU `find` と同じ出力順
- `-regex` / `-iregex` はパス全体にマッチ（暗黙アンカー）
- `-regextype` で `emacs` / `posix-basic`（`ed`/`sed`/`grep`）/ `posix-extended`（デフォルト）等を選択可能
- `-maxdepth` などの位置オプションはテストの後に書いても受理
- `-exec {} +` のコマンドが失敗すると終了コードに反映（`{} \;` は述語の真偽としてのみ機能）
- `-execdir` はファイル名に `./` を前置してコマンドに渡す
- `-printf` は `%-8s` のようなフラグ・フィールド幅、`%D` `%F` `%S` `%B<fmt>`、
  `\a \b \f \v \NNN \c` エスケープに対応
- `-L` でのシンボリックリンク循環を検出して警告
- `-delete` は `.` の削除を拒否し、失敗しても走査を継続（終了コードに反映）

## 注意点

- これは Windows 標準の `find.exe` ではなく、POSIX 風の別実装です
- Windows のパスを比較する内部処理では `/` 区切りに正規化しています
- `-regextype` のデフォルトは GNU（emacs）と異なり `posix-extended` 相当です
- Windows では `-user`/`-perm` などの所有者・パーミッション情報は簡易的なエミュレーションです

## テスト

```bash
cargo test
```

Windows では、開始パス glob 展開と POSIX 風区切りでの `-path` マッチもテストしています。

## ライセンス

MIT
