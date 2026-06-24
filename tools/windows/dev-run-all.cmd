@echo off
setlocal
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0dev-run-all.ps1" %*
if errorlevel 1 exit /b %errorlevel%
