# tee - POSIX準拠出力分岐コマンド（Rust実装）

Windows環境向けのPOSIX準拠`tee`コマンドのRust実装です。GNU拡張も含みます。

## 特徴

- **POSIX準拠**: 標準的な`tee`コマンドの動作を再現
- **GNU拡張対応**: `--output-error`オプションによるエラー処理の制御
- **日本語対応**: ヘルプ・エラーメッセージは日本語
- **glob展開**: Windows環境でも`*.txt`などのパターンを内部展開
- **大文字小文字を非区別**: Windowsらしくファイル名比較はケースインセンシティブ
- **高速**: Rustによる効率的な実装

## インストール

```bash
cargo build --release
cp target/release/tee.exe ~/.local/bin/  # または任意のPATHディレクトリ
```

## 使用法

```
tee [オプション]... [ファイル]...
```

標準入力を標準出力と指定されたファイルにコピーします。

### オプション

| オプション | 説明 |
|-----------|------|
| `-a, --append` | ファイルを上書きせず追記 |
| `-i, --ignore-interrupts` | 割り込みシグナル（SIGINT）を無視 |
| `-p` | 出力エラーの診断を行う（GNU拡張） |
| `--output-error[=MODE]` | 書き込みエラー時の動作を指定（GNU拡張） |

### --output-error のMODE

| MODE | 説明 |
|------|------|
| `warn` | エラー時に警告して続行 |
| `warn-nopipe` | パイプ以外のエラーで警告（デフォルト） |
| `exit` | エラー時に終了 |
| `exit-nopipe` | パイプ以外のエラーで終了 |

## 使用例

### 基本的な使い方
```bash
# 出力を表示しながらファイルに保存
ls -l | tee output.txt

# ファイルに追記
ls -l | tee -a output.txt
```

### 複数ファイルへの出力
```bash
# 2つのファイルに同時出力
ls -l | tee file1.txt file2.txt

# Windowsでもglobパターンで複数ファイルに出力
ls -l | tee logs/*.log
```

### パイプラインでの使用
```bash
# パイプラインの途中で確認・保存
cat data.txt | tee intermediate.txt | sort | uniq

# sortとuniqの間で中間結果を確認
cat data.txt | sort | tee sorted.txt | uniq
```

### ログの保存
```bash
# 標準出力とエラーを両方ログに保存
./script.sh 2>&1 | tee script.log

# makeの出力をログに保存
make 2>&1 | tee build.log
```

### 標準出力への複数回出力
```bash
# - で標準出力を指定（2回表示される）
echo "hello" | tee -
# hello
# hello
```

### 管理者権限でのファイル書き込み
```bash
# sudoで保護されたファイルに書き込み
echo "設定内容" | sudo tee /etc/config > /dev/null
```

### 出力の破棄
```bash
# /dev/null で出力を破棄（ファイルのみ保存）
command | tee /dev/null > file.txt
```

## 応用例

### ログを取りながらリアルタイム監視
```bash
tail -f /var/log/syslog | tee monitoring.log
```

### 処理結果の確認と保存を同時に
```bash
# データ処理パイプラインでの使用
cat raw_data.txt | tr '[:lower:]' '[:upper:]' | tee step1.txt | sort | tee step2.txt | uniq -c
```

### 複数のログファイルに同時記録
```bash
# 日付別とカテゴリ別のログを同時に作成
./app 2>&1 | tee "logs/$(date +%Y%m%d).log" logs/latest.log
```

## 注意事項

- ファイルが指定されない場合、`cat`と同様に標準入力をそのまま標準出力に出力します
- `-`を指定すると標準出力を意味します（2回出力される）
- 書き込みエラーが発生しても、デフォルトでは他のファイルへの出力は継続します
- `-i`オプションはUnix環境でのみ効果があります（WindowsではCtrl+Cは通常通り動作）
- Windowsではシェルが`*.txt`を展開しないことが多いため、この実装は`tee`内部でglob展開します
- glob展開時はWindows向けにファイル名の大文字小文字を区別しません
- globパターンが未一致だった場合は、POSIXシェルの未展開引数に寄せて、そのままファイル名として扱います

## ライセンス

MIT

## 関連

このツールは、Windows環境でUnix系コマンドを使いやすくするプロジェクトの一部です。
`sort`、`uniq`、`tr`と組み合わせて使用することで、強力なテキスト処理パイプラインが構築できます。

```bash
# 単語頻度カウントの途中経過を保存
cat file.txt | tr -cs '[:alnum:]' '\n' | tee words.txt | sort | tee sorted.txt | uniq -c | sort -rn
```
