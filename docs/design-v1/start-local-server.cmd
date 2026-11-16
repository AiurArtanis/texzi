@echo off
setlocal
cd /d "%~dp0"
echo Open http://127.0.0.1:8765/prototype/index.html
py -3 tools\serve.py 8765
pause
endlocal
