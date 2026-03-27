# mv

Rustで実装されたPOSIX準拠の`mv`コマンドです。Windows上でもLinuxと同様の挙動を提供し、内部でglob展開を行います。

## 機能

- ファイルおよびディレクトリの移動と名前変更
- 内部glob展開（case-sensitive、大文字小文字を区別）
- POSIX標準オプションのサポート（`-f`, `-i`）
- GNU拡張オプションのサポート（`-n`, `-u`, `-v`, `-b`, `--backup`, `-S`, `-t`, `-T`, `--strip-trailing-slashes`）
- Windows固有の対応（読み取り専用ファイルの扱い、異なるドライブ間の移動）
- 日本語のエラーメッセージ

## インストール

このプロジェクトをクローンまたはダウンロードし、以下のコマンドでビルドしてください：

```bash
cargo build --release
```

ビルドされたバイナリは `target/release/mv.exe` に生成されます。

## 使用方法

```bash
mv [オプション]... ソース 移動先
mv [オプション]... ソース... ディレクトリ
mv [オプション]... -t ディレクトリ ソース...
```

### オプション

#### POSIX標準オプション
- `-f`, `--force`: 確認なしで上書き（`-i`, `-n` を上書き）
- `-i`, `--interactive`: 上書き前に確認（`-f`, `-n` を上書き）

#### GNU拡張オプション
- `-n`, `--no-clobber`: 既存ファイルを上書きしない（`-f`, `-i` を上書き）
- `-u`, `--update`: ソースが新しい場合、または移動先が存在しない場合のみ移動
- `-v`, `--verbose`: 実行内容を表示
- `-b`: `--backup=existing` と同様
- `--backup[=CONTROL]`: 既存の移動先ファイルをバックアップ
  - `none`, `off`: バックアップを作成しない
  - `numbered`, `t`: 番号付きバックアップを作成
  - `existing`, `nil`: 番号付きバックアップがあれば番号付き、なければ単純
  - `simple`, `never`: 常に単純バックアップを作成
- `-S`, `--suffix=SUFFIX`: バックアップサフィックスを指定
- `-t`, `--target-directory=DIRECTORY`: すべてのソース引数を DIRECTORY に移動
- `-T`, `--no-target-directory`: 移動先を通常のファイルとして扱う
- `--strip-trailing-slashes`: ソース引数から末尾のスラッシュを削除
- `--help`: このヘルプを表示
- `--version`: バージョン情報を表示

## 例

### 基本的な使用
```bash
# ファイル名を変更
mv file.txt newname.txt

# ファイルをディレクトリに移動
mv file.txt dir/

# 複数ファイルをディレクトリに移動
mv file1.txt file2.txt backup/

# glob展開を使用
mv *.txt archive/
```

### オプションの使用
```bash
# 確認付きで移動
mv -i *.txt backup/

# 詳細表示
mv -v olddir newdir

# バックアップを作成して上書き
mv -b file.txt existing.txt

# 更新チェック
mv -u source.txt dest.txt
```

## ビルド要件

- Rust 1.70以上
- Cargo

## ライセンス

MIT

## 貢献

バグ報告や機能リクエストはGitHubのIssueでお願いします。プルリクエストも歓迎です。

## 作者

Rustコミュニティ</content>
<parameter name="filePath">d:\home\source\rust\tools\mv\README.md
