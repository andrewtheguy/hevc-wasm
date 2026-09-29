#!/usr/bin/env bash
# The Emscripten half of build.sh, run inside the emsdk container.
set -euo pipefail

src=/src/build/FFmpeg-$FFMPEG_COMMIT
prefix=/src/build/prefix
out=/src/build/out

# Every optimization the target takes: -O3 with link-time optimization across
# libavcodec and the shim, SIMD128 (FFmpeg's own IDCT and SAO kernels, the fork's
# block copy, interpolation filters and residual add, and the autovectorizer
# everywhere else), bulk memory for memcpy and memset, and pthreads
# for the decoder's slice threads.
FLAGS="-O3 -flto -msimd128 -mbulk-memory -mnontrapping-fptoint -pthread -DNDEBUG"

rm -rf "$src" "$prefix" "$out" /src/dist
mkdir -p "$HOME"
tar -xzf "/src/build/ffmpeg-$FFMPEG_COMMIT.tar.gz" -C /src/build

cd "$src"

emconfigure ./configure \
  --prefix="$prefix" \
  --target-os=none \
  --arch=wasm32 \
  --enable-cross-compile \
  --cc=emcc --cxx=em++ --ar=emar --ranlib=emranlib --nm=emnm \
  --disable-everything \
  --disable-autodetect \
  --disable-programs \
  --disable-doc \
  --disable-debug \
  --disable-stripping \
  --disable-network \
  --disable-avdevice --disable-avformat --disable-avfilter \
  --disable-swscale --disable-swresample \
  --disable-runtime-cpudetect \
  --disable-inline-asm \
  --enable-pthreads \
  --enable-simd128 \
  --enable-decoder=hevc \
  --enable-parser=hevc \
  --optflags="-O3" \
  --extra-cflags="$FLAGS" \
  --extra-ldflags="$FLAGS"

# configure knows wasm only by name, and enables none of the arch traits that
# pick libavcodec's faster C paths. WebAssembly has all three: loads at any
# alignment, native 64-bit integers, and a count-leading-zeros instruction.
sed -i -E 's/^#define HAVE_(FAST_UNALIGNED|FAST_64BIT|FAST_CLZ) 0$/#define HAVE_\1 1/' config.h

grep -E '^#define (HAVE_FAST_UNALIGNED|HAVE_FAST_64BIT|HAVE_FAST_CLZ|HAVE_SIMD128|HAVE_PTHREADS|HAVE_THREADS|CONFIG_HEVC_DECODER) ' config.h
grep -q '^#define HAVE_SIMD128 1' config.h
grep -q '^#define HAVE_PTHREADS 1' config.h

emmake make -j"$(nproc)" install-libs install-headers

mkdir -p "$out"
cd /src
# The page runs the decoder in a worker of its own, and each slice thread is a
# pthread worker the module starts before it resolves (PTHREAD_POOL_SIZE, read
# from the `threads` the page instantiates it with).
emcc $FLAGS \
  -I"$prefix/include" \
  src/decoder.c \
  "$prefix/lib/libavcodec.a" "$prefix/lib/libavutil.a" \
  -o "$out/hevc.js" \
  -sMODULARIZE=1 \
  -sEXPORT_ES6=1 \
  -sEXPORT_NAME=createHevcModule \
  -sENVIRONMENT=worker \
  -sEXPORTED_FUNCTIONS=_hevc_create,_hevc_input,_hevc_decode,_hevc_picture,_hevc_destroy \
  -sEXPORTED_RUNTIME_METHODS=HEAPU8,HEAP32,wasmMemory \
  -sINCOMING_MODULE_JS_API=locateFile,print,printErr \
  -sPTHREAD_POOL_SIZE='Module["threads"]' \
  -sPTHREAD_POOL_SIZE_STRICT=2 \
  -sDEFAULT_PTHREAD_STACK_SIZE=1MB \
  -sSTACK_SIZE=1MB \
  -sALLOW_MEMORY_GROWTH=1 \
  -sINITIAL_MEMORY=128MB \
  -sMAXIMUM_MEMORY=4GB \
  -sMALLOC=mimalloc \
  -sFILESYSTEM=0 \
  -sASSERTIONS=0 \
  -sSUPPORT_ERRNO=0 \
  -sTEXTDECODER=2

# The same decoder under Node, for bench/bench.c: timing and a digest per picture.
emcc $FLAGS \
  -I"$prefix/include" \
  src/decoder.c bench/bench.c \
  "$prefix/lib/libavcodec.a" "$prefix/lib/libavutil.a" \
  -o "$out/bench.js" \
  -sENVIRONMENT=node \
  -sNODERAWFS=1 \
  -sPTHREAD_POOL_SIZE=16 \
  -sDEFAULT_PTHREAD_STACK_SIZE=1MB \
  -sSTACK_SIZE=1MB \
  -sALLOW_MEMORY_GROWTH=1 \
  -sINITIAL_MEMORY=128MB \
  -sMAXIMUM_MEMORY=4GB \
  -sMALLOC=mimalloc \
  -sEXIT_RUNTIME=1

# The release: the two files the page loads, and nothing else. GNU tar with fixed
# names, owners and times, so one build's archive is byte for byte the next's.
mkdir -p /src/dist
tar --sort=name --owner=0 --group=0 --numeric-owner --mtime=@0 \
  -C "$out" -cf - hevc.js hevc.wasm | gzip -9n >"/src/dist/hevc-wasm-v$VERSION.tar.gz"
