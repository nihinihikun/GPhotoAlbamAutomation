@echo off
setlocal
cd /d "%~dp0" || exit /b 1
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0setup-portable.ps1"
set "GPAE_EXIT_CODE=%ERRORLEVEL%"
pause
exit /b %GPAE_EXIT_CODE%
