# top

Windows 向けの `top` 実装です。Unix 系 `top` の画面構成と操作感を意識しつつ、Windows のプロセス情報をリアルタイム表示します。

## 特徴

- 対話モードとバッチモードの両方に対応
- CPU 使用率、メモリ使用率、プロセス数、稼働時間を継続表示
- `%CPU` `%MEM` `TIME` `PID` `USER` `RES` `COMMAND` などでソート可能
- ユーザー名や PID によるフィルタリングをサポート
- プロセス終了や優先度変更などの対話操作に対応
- `--secure` により破壊的操作を無効化できる

## 使い方

```powershell
cargo run --
cargo run -- -d 0.5
cargo run -- -b -n 10
cargo run -- -u Administrator
cargo run -- -p 1234,5678
cargo run -- -o %MEM
```

## 主なオプション

- `-b, --batch`
  非対話モードで出力します
- `-d SEC, --delay=SEC`
  更新間隔を秒で指定します
- `-n NUM, --iterations=NUM`
  更新回数を指定します
- `-u USER, --user=USER`
  指定ユーザーのプロセスだけを表示します
- `-p PID, --pid=PID`
  指定 PID だけを表示します
- `-o FIELD, --sort=FIELD`
  ソートフィールドを変更します
- `-H, --threads`
  スレッド情報を表示します
- `-s, --secure`
  キルや優先度変更を無効化します

## 対話モードの主なキー

- `q`, `Esc`, `Ctrl+C`
  終了します
- `P`, `M`, `T`, `N`
  ソート基準を切り替えます
- `R`
  ソート順を反転します
- `↑`, `↓`, `j`, `k`
  選択を移動します
- `K`, `F9`
  選択プロセスを終了します
- `r`
  優先度を変更します
- `d`, `s`
  更新間隔を変更します
- `h`, `?`, `F1`
  ヘルプを表示します

## 互換性について

この実装は Windows API を使って情報を取得します。Linux の `top` と完全一致ではありませんが、日常的なプロセス監視に必要な表示と操作を一通り備えています。

## テスト

```powershell
cargo test
```

## ライセンス

MIT
