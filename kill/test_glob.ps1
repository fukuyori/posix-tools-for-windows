# glob 展開テストスクリプト

Write-Host "=== Glob Expansion Test ===" -ForegroundColor Cyan
Write-Host ""

# テスト用プロセスの確認
Write-Host "1. テスト用プロセス（notepad）の確認:"
Get-Process notepad -ErrorAction SilentlyContinue | ForEach-Object { Write-Host "   PID: $($_.Id), Name: $($_.Name)" }
Write-Host ""

# glob パターンマッチングテスト（実行しない、期待値表示）
Write-Host "2. Glob パターンマッチングの期待値:"
Write-Host "   パターン: note*.exe"
Write-Host "   -> マッチするプロセス: notepad.exe"
Write-Host ""
Write-Host "   パターン: notepad.exe"
Write-Host "   -> マッチするプロセス: notepad.exe"
Write-Host ""
Write-Host "   パターン: note?ad.exe"
Write-Host "   -> マッチするプロセス: notepad.exe"
Write-Host ""

# テスト用プロセスをクリーンアップ
Write-Host "3. テスト用プロセスをクリーンアップ:"
Get-Process notepad -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
Write-Host "   Notepad プロセスを終了しました"
Write-Host ""

Write-Host "=== Test Complete ===" -ForegroundColor Green
