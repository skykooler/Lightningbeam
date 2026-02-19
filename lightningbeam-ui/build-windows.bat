@echo off
REM Build script for Windows
REM Requires: FFmpeg 8.0.0 dev files in C:\ffmpeg, LLVM installed

REM FFmpeg location (headers + libs + DLLs)
if not defined FFMPEG_DIR set FFMPEG_DIR=C:\ffmpeg

REM LLVM/libclang for bindgen (ffmpeg-sys-next)
if not defined LIBCLANG_PATH set LIBCLANG_PATH=C:\Program Files\LLVM\bin

REM Validate prerequisites
if not exist "%FFMPEG_DIR%\include\libavcodec\avcodec.h" (
    echo ERROR: FFmpeg dev files not found at %FFMPEG_DIR%
    echo Download FFmpeg 8.0.0 shared+dev from https://github.com/GyanD/codexffmpeg/releases
    echo and extract to %FFMPEG_DIR%
    exit /b 1
)

if not exist "%LIBCLANG_PATH%\libclang.dll" (
    echo ERROR: LLVM/libclang not found at %LIBCLANG_PATH%
    echo Install with: winget install LLVM.LLVM
    exit /b 1
)

echo Building Lightningbeam Editor...
echo   FFMPEG_DIR=%FFMPEG_DIR%
echo   LIBCLANG_PATH=%LIBCLANG_PATH%

cargo build --package lightningbeam-editor %*
