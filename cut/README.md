# cut

Windows 向けの `cut` 実装です。POSIX `cut` の基本挙動を保ちつつ、Windows 上でも Linux に近い感覚で使えるように内部 glob 展開やマルチバイト文字対応を加えています。

## 特徴

- `-b` `-c` `-f` の 3 モードに対応
- `-c` は文字位置ベースで処理し、マルチバイト文字を扱える
- `-d` `-s` によるフィールド切り出しをサポート
- `--complement` `--output-delimiter` `-z` などの GNU 拡張に対応
- Windows 上でも内部 glob 展開により `*.txt` のような指定が使える

## 使い方

```powershell
cargo run -- -f1 file.txt
cargo run -- -d, -f2 file.csv
cargo run -- -c1-10 file.txt
cargo run -- --complement -f1 data.tsv
```

## 主なオプション

- `-b, --bytes=LIST`
  バイト位置で選択します
- `-c, --characters=LIST`
  文字位置で選択します
- `-f, --fields=LIST`
  フィールド単位で選択します
- `-d, --delimiter=DELIM`
  フィールド区切り文字を指定します
- `-s, --only-delimited`
  区切り文字を含まない行を出力しません
- `--complement`
  選択範囲を反転します
- `--output-delimiter=STR`
  出力時の区切り文字を変更します
- `-z, --zero-terminated`
  行区切りを NUL にします

## 範囲指定

- `N`
  N 番目だけを選択
- `N-`
  N 番目から末尾まで
- `-M`
  先頭から M 番目まで
- `N-M`
  N 番目から M 番目まで
- `1,3,5-7`
  カンマ区切りで複数指定

## 互換性について

この実装は Windows 向けです。POSIX `cut` の主要機能には対応していますが、glob 展開はシェルではなくコマンド自身が行います。

## テスト

```powershell
cargo test
```

## ライセンス

MIT
