@echo off
setlocal
cd /d "%~dp0" || exit /b 1
if not exist config.json (
  echo config.json not found. Run SETUP.bat.
  pause
  exit /b 1
)
if exist "%USERPROFILE%\.cargo\bin\cargo.exe" set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
if defined CARGO_HOME if exist "%CARGO_HOME%\bin\cargo.exe" set "PATH=%CARGO_HOME%\bin;%PATH%"
where cargo.exe >nul 2>&1
if errorlevel 1 (
  echo Cargo not found. Run SETUP.bat.
  pause
  exit /b 1
)
powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File "%~dp0run.ps1"
set "GPAE_EXIT_CODE=%ERRORLEVEL%"
if "%GPAE_EXIT_CODE%"=="2" echo Some results could not be verified. See the PARTIAL report.
if "%GPAE_EXIT_CODE%"=="1" echo Export failed. See the error above and any report in the output folder.
pause
exit /b %GPAE_EXIT_CODE%
