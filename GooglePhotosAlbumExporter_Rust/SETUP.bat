@echo off
setlocal
cd /d "%~dp0" || exit /b 1
if not exist config.json (
  copy config.example.json config.json >nul || exit /b 1
  echo Set account and output_dir, then save and close Notepad.
  notepad.exe config.json
  if errorlevel 1 (pause & exit /b 1)
)
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0setup.ps1"
set "GPAE_EXIT_CODE=%ERRORLEVEL%"
pause
exit /b %GPAE_EXIT_CODE%
