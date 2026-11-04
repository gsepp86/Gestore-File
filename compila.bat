@echo off
echo Compilazione in corso (la prima volta ci vogliono alcuni minuti)...
cargo build --release
if errorlevel 1 (
  echo.
  echo ERRORE: hai installato Rust da https://rustup.rs ?
  pause
  exit /b 1
)
echo.
echo FATTO! Il programma e' in: target\release\gestore-file.exe
pause
