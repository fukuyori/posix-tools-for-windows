# awk-rs

Windows 向けの POSIX AWK 互換実装です。  
Windows の使い勝手は維持しつつ、AWK 言語仕様はできるだけ POSIX に寄せています。

## 特徴

- `getline`, `split`, `sub`, `gsub`, `match`, `nextfile`, `fflush`, `system` を実装
- `ARGC`, `ARGV`, `ENVIRON`, `RSTART`, `RLENGTH`, `OFMT` を実装
- `BEGIN` で変更した `ARGV` を実際の入力ファイル列に反映
- Windows では CLI 引数の glob 展開を awk 側で補完
- ファイル名比較は Windows 向けに case-insensitive
- 正規表現は独自の **C ロケール固定 POSIX ERE** エンジンを使用

## 実装済み機能

- パターン: `BEGIN`, `END`, 正規表現パターン, 式パターン, 範囲パターン
- アクション: `print`, `printf`, `if`, `while`, `do/while`, `for`, `for (k in a)`, `break`, `continue`, `next`, `nextfile`, `exit`, `return`, `delete`
- 演算子: 算術, 比較, 論理, 正規表現マッチ, 三項演算子, 代入, インクリメント/デクリメント
- データモデル: フィールドアクセス, 連想配列, ユーザー定義関数, 文字列連結
- 入出力: `>`, `>>`, `|`, `getline`, `close`, `fflush`, `system`

## 主な組み込み関数

- 文字列: `length`, `substr`, `index`, `split`, `sub`, `gsub`, `match`, `sprintf`, `tolower`, `toupper`
- 数学: `sin`, `cos`, `atan2`, `exp`, `log`, `sqrt`, `int`, `rand`, `srand`
- その他: `system`

## 使い方

```bash
# ビルド
cargo build --release

# 実行
awk 'プログラム' [ファイル...]
awk -f プログラムファイル [ファイル...]
awk -F 区切り文字 'プログラム' [ファイル...]
awk -v 変数=値 'プログラム' [ファイル...]
```

Windows では `*.txt` のようなファイル引数を `awk` 側で glob 展開します。  
マッチしない場合は POSIX シェル寄りにリテラルのまま扱います。

## 使用例

```bash
# 全行を出力
echo -e "hello\nworld" | awk '{ print }'

# 特定のフィールドを出力
echo "John 25 Engineer" | awk '{ print $1, $3 }'

# 数値の合計
echo -e "10\n20\n30" | awk '{ sum += $1 } END { print sum }'

# パターンでフィルタリング
echo -e "apple\nbanana\napricot" | awk '/^a/'

# フィールド区切り文字を指定
echo "a,b,c" | awk -F, '{ print $2 }'

# 平均を計算
awk '{ sum += $1 } END { print "平均:", sum/NR }' numbers.txt

# 出現回数をカウント
awk '{ count[$1]++ } END { for (w in count) print w, count[w] }' words.txt

# ファイルへの出力リダイレクト
awk '{ print $1 > "output.txt" }' input.txt

# ファイルへの追記
awk '{ print $1 >> "log.txt" }' input.txt

# コマンドへパイプ出力
awk '{ print | "sort" }' input.txt

# ファイルからgetlineで読み込み
awk 'BEGIN { while ((getline line < "data.txt") > 0) print line }'

# コマンドからgetlineで読み込み
awk 'BEGIN { while (("date" | getline d) > 0) print d }'

# BEGIN で入力ファイル列を書き換え
awk 'BEGIN { ARGV[1] = "other.txt" } { print FILENAME, $0 }' input.txt

# nextfile で現在のファイルの残りを飛ばす
awk '/^#/ { nextfile } { print }' file1.txt file2.txt
```

## Windows用にビルド

```bash
# Linux上でWindowsにクロスコンパイル
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu

# Windows上でビルド
cargo build --release
```

## プロジェクト構造

```
awk-rs/
├── Cargo.toml          # プロジェクト設定
├── src/
│   ├── main.rs         # エントリーポイント、CLI処理
│   ├── lexer.rs        # 字句解析器
│   ├── parser.rs       # 構文解析器（トークン → AST）
│   ├── ast.rs          # AST定義
│   ├── interpreter.rs  # インタプリタ/評価器
│   ├── regex_compat.rs # Cロケール固定 POSIX ERE エンジン
│   ├── value.rs        # AWKの値と変数
│   └── builtins.rs     # 組み込み関数
```

## POSIX互換の方針

- AWK 言語仕様はできるだけ POSIX awk に合わせる
- 正規表現は **C ロケール相当の POSIX ERE** として解釈する
- POSIX 文字クラス `[[:alpha:]]` などは ASCII ベースで判定する
- Windows 固有部分として、ファイル名比較は case-insensitive にする

## 制限事項

- ロケール依存の照合順序・同値クラスまでは未対応
- `[[.ch.]]` のような collating symbol は未対応
- `[[=a=]]` のような equivalence class は未対応
- regex エンジンはかなり POSIX ERE に寄せているが、厳密な全互換を保証するものではない

## テスト

```bash
# 通常テスト
cargo test

# 重めの regex stress test も含めて実行
cargo test -- --ignored
```

## ライセンス

MIT
