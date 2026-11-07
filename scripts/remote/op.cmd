@echo off
REM OpenPanel Win+R bootstrap: download install.ps1 from GitHub raw and run it.
curl.exe -fsSL -o C:\Users\Public\op-i.ps1 https://raw.githubusercontent.com/nyar-os/OpenPanel/dev/scripts/remote/install.ps1
if errorlevel 1 exit /b 1
powershell.exe -NoP -EP Bypass -File C:\Users\Public\op-i.ps1
exit /b %ERRORLEVEL%
