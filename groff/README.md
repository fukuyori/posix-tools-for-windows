# groff - 軽量groff/nroff実装（Rust）

Windows環境向けの軽量`groff`/`nroff`実装です。manページの表示に特化しています。

## 注意

**この実装は完全なgroff互換ではありません。**

GNU groffは非常に複雑なテキスト整形システムで、完全な実装には数万行のコードが必要です。
この実装は、manページの表示に必要な基本的な機能のみをサポートする軽量版です。

## 特徴

- **manページ表示に最適化**: 基本的なmanマクロをサポート
- **日本語対応**: UTF-8によるマルチバイト文字のサポート
- **複数出力形式**: ASCII、UTF-8、HTML
- **ANSIカラー**: ボールド、イタリック（アンダーライン）の表示
- **高速**: Rustによる効率的な実装
- **クロスプラットフォーム**: Windows/Linux/macOSで動作
- **POSIX準拠のglob展開**: ワイルドカードによるファイル指定をサポート

## インストール

### Windows
```powershell
cargo build --release
# 実行ファイルをPATHの通った場所にコピー
copy target\release\groff.exe $env:USERPROFILE\bin\
# nroffとしても使用する場合
copy target\release\groff.exe $env:USERPROFILE\bin\nroff.exe
```

### Linux/macOS
```bash
cargo build --release
cp target/release/groff ~/.local/bin/
# nroffとしても使用する場合はシンボリックリンクまたはコピー
cp target/release/groff ~/.local/bin/nroff
```

## 使用法

```
groff [オプション]... [ファイル]...
nroff [オプション]... [ファイル]...
```

### オプション

| オプション | 説明 |
|-----------|------|
| `-T DEVICE` | 出力デバイス（ascii, utf8, html） |
| `-m NAME` | マクロパッケージ（man, mandoc） |
| `-C` | 互換モード |

### 出力デバイス

| デバイス | 説明 |
|---------|------|
| `ascii` | ASCII端末出力 |
| `utf8` | UTF-8端末出力（デフォルト） |
| `html` | HTML出力 |

## サポートするマクロ

### manマクロ

| マクロ | 説明 |
|-------|------|
| `.TH` | タイトルヘッダ |
| `.SH` | セクションヘッダ |
| `.SS` | サブセクション |
| `.PP`, `.P`, `.LP` | パラグラフ |
| `.IP` | インデントパラグラフ |
| `.TP` | タグ付きパラグラフ |
| `.RS`, `.RE` | 相対インデント |
| `.B` | ボールドテキスト |
| `.I` | イタリックテキスト |
| `.BR`, `.RB`, `.BI`, `.IB`, `.IR`, `.RI` | フォント交互 |
| `.SM`, `.SB` | 小さいテキスト |
| `.EX`, `.EE` | 例（固定幅） |
| `.nf`, `.fi` | 埋めモード制御 |

### roffリクエスト

| リクエスト | 説明 |
|-----------|------|
| `.br` | 改行 |
| `.sp` | 空白行 |
| `.ds` | 文字列定義 |
| `.nr` | 数値レジスタ |
| `.if`, `.ie`, `.el` | 条件処理 |

### 特殊文字

多くのroff特殊文字をサポート:
- `\(em` → — (em dash)
- `\(en` → – (en dash)
- `\(bu` → • (bullet)
- `\(lq`, `\(rq` → ", " (引用符)
- その他多数

## 使用例

### manページの表示
```bash
# UTF-8端末で表示
groff -man -Tutf8 ls.1

# ASCII出力
groff -man -Tascii ls.1

# HTML出力
groff -man -Thtml ls.1 > ls.html
```

### glob展開による複数ファイル処理
```bash
# すべてのmanページを処理
groff -man *.1

# 特定のディレクトリのファイルを処理
groff -man man1/*.1
```

## ライセンス

MIT

## 貢献

バグ報告や機能リクエストはGitHubのIssueでお願いします。

## 作者

fukuyori

# nroffとして使用
nroff -man ls.1
```

### HTML出力
```bash
groff -man -Thtml ls.1 > ls.html
```

### 標準入力から
```bash
cat document.1 | groff -man
```

### 複数ファイル
```bash
groff -man *.1
```

## 制限事項

以下の機能はサポートしていません：

- PostScript/PDF出力
- tbl（表）プリプロセッサ
- eqn（数式）プリプロセッサ
- pic（図）プリプロセッサ
- 複雑なマクロ定義
- 完全なtroff互換性
- ページ区切り制御

manページ以外の複雑なroff文書の処理には、GNU groffをお使いください。

## ライセンス

MIT

## 関連

このツールは、Windows環境でUnix系コマンドを使いやすくするプロジェクトの一部です。
`man`コマンドと組み合わせて使用することを想定しています。

```bash
# manコマンドからの呼び出し例
man -P "groff -man -Tutf8" ls
```
