@echo off
setlocal
cd /d "%~dp0" || exit /b 1
if exist "%USERPROFILE%\.cargo\bin\cargo.exe" set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
if defined CARGO_HOME if exist "%CARGO_HOME%\bin\cargo.exe" set "PATH=%CARGO_HOME%\bin;%PATH%"
where cargo.exe >nul 2>&1
if errorlevel 1 (echo Cargo not found. Run SETUP.bat. & exit /b 1)
if /i "%~1"=="--live" (
  cargo.exe run --release --locked -- probe --config config.json
  exit /b
)
cargo.exe fmt --all -- --check || exit /b 1
cargo.exe test --locked || exit /b 1
cargo.exe clippy --locked --all-targets -- -D warnings || exit /b 1
cargo.exe build --release --locked || exit /b 1
where node.exe >nul 2>&1
if errorlevel 1 (
  echo Install Node.js to run the JavaScript adapter tests.
  exit /b 1
)
node.exe --check src/photos.js || exit /b 1
node.exe --test tests/photos_adapter.test.cjs || exit /b 1
if exist config.json cargo.exe run --release --locked -- check --config config.json
if errorlevel 1 exit /b 1
echo Verification passed.
