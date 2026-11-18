@echo off
setlocal
cd /d "%~dp0" || exit /b 1
if not exist gpae.exe (
  echo gpae.exe not found. Extract the complete ZIP before running this launcher.
  pause
  exit /b 1
)
if not exist config.json (
  echo config.json not found. Run SETUP.bat first.
  pause
  exit /b 1
)
"%~dp0gpae.exe" run --config "%~dp0config.json"
set "GPAE_EXIT_CODE=%ERRORLEVEL%"
if "%GPAE_EXIT_CODE%"=="2" echo Some results could not be verified. See the PARTIAL report.
if "%GPAE_EXIT_CODE%"=="1" echo Export failed. See the error above and any report in the output folder.
if "%GPAE_EXIT_CODE%"=="3" echo Export cancelled.
pause
exit /b %GPAE_EXIT_CODE%
