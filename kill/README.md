# kill - Windows 用 POSIX 互換プロセス終了ツール

Windows 上で POSIX 準拠の `kill` コマンド的な動作を実現するツール。

## 機能

- **PID による終了** - 数値 PID を指定して直接プロセス終了
- **プロセス名による終了** - 実行可能ファイル名を指定してプロセス終了
- **シグナル指定** - `-s TERM`, `-9`, `-KILL` など複数の指定方法
- **シグナル一覧表示** - `-l` で変換、`-L` でテーブル表示
- **Glob パターン展開** ⭐ - `note*.exe`, `test?.exe` など Windows 上でワイルドカード対応

## Glob 機能

Windows 環境では、シェル側が glob 展開をしないことが多いため、kill 自体が glob パターンマッチングに対応しています。

### サポートされるパターン

| パターン | 説明 | 例 |
|---------|------|-----|
| `*` | 0 文字以上にマッチ | `note*.exe` → `notepad.exe`, `notepad2.exe` |
| `?` | 1 文字にマッチ | `test?.exe` → `test1.exe`, `testa.exe` |
| `[...]` | 文字セット | (基本実装) |

### 使用例

```powershell
# プロセス名パターンで複数終了
kill note*.exe          # notepad.exe で始まるプロセスを終了

kill test?.exe          # test1.exe, test2.exe などを終了

# 数値PIDは glob 対象外
kill 1234               # そのままPIDとして処理

# プロセス名の完全一致
kill notepad.exe        # glob パターンでなければ完全一致で検索
```

## ビルド

```bash
cargo build --release
```

実行ファイル: `target/release/kill.exe`

## 使い方

```bash
kill [-s シグナル | -シグナル] PID...
kill -l [シグナル]...
kill -L
```

### オプション

| オプション | 説明 |
|-----------|------|
| `-s シグナル` | シグナルを名前または番号で指定 |
| `-シグナル名` | 短形式: `-TERM`, `-KILL`, `-HUP` |
| `-シグナル番号` | 短形式: `-9`, `-15` |
| `-l [シグナル]` | シグナル変換 (番号↔名前) |
| `-L` | シグナル一覧をテーブル形式で表示 |
| `--help` | ヘルプ表示 |
| `--version` | バージョン表示 |

### シグナル一覧

よく使われるシグナル:

- `HUP (1)` - ハングアップ
- `INT (2)` - 割り込み（Ctrl+C相当）
- `QUIT (3)` - 終了
- `KILL (9)` - 強制終了（捕捉不可）
- `TERM (15)` - 終了要求（デフォルト）

その他 31 シグナルに対応。

## 使用例

```bash
# 基本的な使い方
kill 1234                    # PID 1234 に SIGTERM を送信
kill -9 1234                 # PID 1234 を強制終了
kill -KILL 1234              # 同上（シグナル名で指定）
kill -s TERM 1234            # 同上（-s オプション）

# 複数プロセス終了
kill 1234 5678               # 複数PID を一度に指定

# Glob パターン（Windows拡張機能）
kill note*.exe               # notepad.exe で始まるプロセスを全て終了
kill test?.exe               # test1.exe, testa.exe を終了

# プロセス名で指定
kill notepad.exe             # notepad.exe プロセスを終了

# シグナル情報表示
kill -l                      # シグナル名一覧表示
kill -l 9                    # シグナル番号 9 の名前を表示
kill -l KILL                 # シグナル KILL の番号を表示
kill -L                      # テーブル形式で一覧表示
```

## 注意

- **Windows での動作** - すべてのシグナルは Windows API の `TerminateProcess` として処理されます
- **プロセスグループ** - 負の PID（プロセスグループ）は Windows でサポートされていません
- **POSIX 互換性** - 全足りない部分はありますが、基本的な `kill` の動作に準拠

## 実装について

このツールは以下の Windows API を使用:
- `CreateToolhelp32Snapshot` - プロセス列挙
- `Process32First/Next` - プロセス情報取得
- `OpenProcess/TerminateProcess` - プロセス終了
