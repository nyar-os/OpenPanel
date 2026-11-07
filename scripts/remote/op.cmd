@echo off
REM OpenPanel Win+R bootstrap via jsDelivr (raw.githubusercontent.com GET flaky in CN).
curl.exe -fsSL -A "Mozilla/5.0" -o C:\Users\Public\op-i.ps1 https://cdn.jsdelivr.net/gh/nyar-os/OpenPanel@dev/scripts/remote/install.ps1
if errorlevel 1 exit /b 1
powershell.exe -NoP -EP Bypass -File C:\Users\Public\op-i.ps1
exit /b %ERRORLEVEL%
