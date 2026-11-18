@echo off
setlocal
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0uninstall.ps1"
set "GPAE_EXIT_CODE=%ERRORLEVEL%"
echo.
pause
exit /b %GPAE_EXIT_CODE%
