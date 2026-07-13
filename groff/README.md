# groff - groff/nroff 実装（Rust）

Windows 環境向けの `groff`/`nroff` 実装です。roff 言語のコア（レジスタ・文字列・
マクロ定義・条件・数値式・エスケープ）と man マクロパッケージを実装しており、
man ページのレンダリングで GNU groff と同一の出力を目指しています。

## 互換性の実測

GNU groff 1.23.0（`groff -man -Tutf8`）と Ubuntu の実際の man ページで
出力を比較検証しています:

- **246 ページ中 214 ページ（87%）がバイト単位で完全一致**
  （coreutils 全般、grep、tar、find、man-db、pod2man 生成ページ、
  docbook 生成ページなど）
- 残りは bash / curl / dash など、独自マクロを多用する巨大ページや
  tbl / eqn（プリプロセッサ）を必要とするページ

## 実装している機能

### roff 言語コア

- レジスタ（`.nr`、`\n`、自動増分、組み込みレジスタ）
- 文字列（`.ds` / `.as`、`\*`、再帰展開）
- マクロ定義（`.de` / `.am`、`\$1`〜`\$9`、`\$*`、`.shift`、`.return`）
- 条件（`.if` / `.ie` / `.el`、`\{ ... \}` ブロック、n/t/d/r/文字列比較/数値式）
- 数値式（優先順位なし左結合、単位 u n m v i c p P、`\w'...'`）
- `.so`（相対パス解決付き）、`.ig`、`.rm` / `.rn` / `.als`、`.nop`、`.do`
- 行末 `\` による行継続、`#n` 等の roff の細部

### 整形エンジン

- 両端揃え（GNU と同じ余白配分・行ごとの交互方向）
- **Knuth-Liang ハイフネーション**（groff 同梱の hyphen.en / hyphenex.en
  パターンを使用。明示ハイフン・`\%`・`\-`・`.hy`/`.nh` の区別を含めて
  GNU の挙動に一致）
- 文末二重スペース（`\&` / `\|` による打ち消しも GNU 準拠）
- インデント・一時インデント・センタリング（`.ce`）・行長（`.ll`）
- タブストップ（`.ta`、デフォルト 0.5i）
- `\f` フォント切替（SGR 出力: 太字 = `\e[1m`、イタリック = `\e[4m`）
- 特殊文字 `\(xx` / `\[xxx]`（`\[uXXXX]` ユニコード指定を含む）

### man マクロ

`.TH`（3 分割ヘッダ/フッタ、セクション名デフォルト）、`.SH` / `.SS`、
`.PP` / `.P` / `.LP`、`.IP` / `.TP` / `.TQ` / `.HP`（ぶら下げタグ、
長いタグの折り返し）、`.PD`、`.RS` / `.RE`、
`.B` / `.I` / `.BR` / `.RB` / `.IR` / `.RI` / `.BI` / `.IB` / `.SM` / `.SB`
（交互フォントの密着連結、引用符内スペースの扱いを含む）、
`.EX` / `.EE`、`.UR` / `.UE`、`.MT` / `.ME`、`.SY` / `.YS`、`.OP`、`.DT`

見出し直後の空行抑制（no-space モード）や `.IP`/`.TP` による調整モードの
リセットなど、GNU an.tmac の細かい挙動も再現しています。

## 使い方

```powershell
groff -man -Tutf8 ls.1              # man ページを表示
groff -man ls.1 | less -R           # ページャで表示
groff -man -rLL=100n wide.1         # 行長 100 桁
groff -man -c plain.1               # SGR 装飾なし
nroff -man ls.1                     # nroff として呼び出し
```

## オプション

- `-T DEVICE` — utf8（デフォルト）/ ascii / latin1 / html
- `-m NAME` — マクロパッケージ（man は組み込み）
- `-r REG=EXPR` / `-d STR=VAL` — レジスタ / 文字列の事前定義
- `-c` — SGR エスケープを無効化（環境変数 GROFF_NO_SGR も有効）
- `-a` — テキスト近似出力
- `-z` — 整形のみ（出力抑制）
- `-k -t -e -p -s -R -C -w -W` など — 互換のため受理

## ビルド

```powershell
cargo build --release
```

生成物は `target/release/groff.exe` です。`nroff.exe` という名前でコピー
すると nroff として動作します。

## 既知の制限

- tbl / eqn / pic などのプリプロセッサは未実装（`.TS` テーブル等は崩れます）
- mdoc マクロパッケージ（BSD 系 man ページ）は未対応
- 縦方向の高度な機能（トラップ、diversion、ページネーション）は省略
  （man 表示では連続レンダリングが標準のため実用上の影響はほぼありません）

## ハイフネーションパターンのライセンス

`src/hyphen_en.txt` / `src/hyphenex_en.txt` は groff 1.23.0 に同梱される
米語ハイフネーションパターン（Gerard D.C. Kuiken 氏および TeX Users Group
による）で、各ファイル内の著作権表示のとおり自由に複製・再配布できます。

## テスト

```powershell
cargo test
```

パーサー・エスケープ・タグ配置・両端揃え・分綴・条件・マクロ展開など
28 件のユニットテストがあります。

## ライセンス

MIT
