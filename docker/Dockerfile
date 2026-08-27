# syntax=docker/dockerfile:1
#
# Windows build box for a Windows-only app.
#
# The GNU target links the executable with mingw and never touches a Windows
# toolchain, so the whole build runs on Linux. That is what makes a container
# useful here: no Visual Studio, no Smart App Control interrupting the build
# scripts, and the same output on any machine.
#
#   docker build -f docker/Dockerfile --target artifact --output out .
#
# leaves battery-hub.exe and WebView2Loader.dll in ./out. Both still have to be
# signed on Windows afterwards -- see README.

FROM node:22-bookworm-slim AS frontend
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY tsconfig.json tsconfig.node.json vite.config.ts index.html ./
COPY src ./src
COPY public ./public
RUN npm run build

FROM rust:1-bookworm AS backend
RUN apt-get update \
 && apt-get install -y --no-install-recommends gcc-mingw-w64-x86-64 \
 && rm -rf /var/lib/apt/lists/*
RUN rustup target add x86_64-pc-windows-gnu
WORKDIR /app
COPY src-tauri ./src-tauri
COPY --from=frontend /app/dist ./dist
# The GNU linker cannot produce the staticlib and cdylib the MSVC bundle asks
# for, so the library is built as a plain rlib and only the binary is linked.
# custom-protocol is not optional: without it the window opens on "this page
# can't be reached" instead of the assets compiled into the binary.
RUN sed -i 's/^crate-type = .*/crate-type = ["rlib"]/' src-tauri/Cargo.toml \
 && cd src-tauri \
 && cargo build --release --target x86_64-pc-windows-gnu \
      --bin battery-hub --features tauri/custom-protocol

# Lint against the Windows target rather than the host's, so the cfg(windows)
# code -- which is nearly all of the device layer -- is the code being checked.
FROM backend AS test
RUN rustup component add clippy
RUN cd src-tauri \
 && cargo clippy --target x86_64-pc-windows-gnu --lib --tests -- -D warnings

FROM scratch AS artifact
COPY --from=backend /app/src-tauri/target/x86_64-pc-windows-gnu/release/battery-hub.exe /
COPY --from=backend /app/src-tauri/target/x86_64-pc-windows-gnu/release/WebView2Loader.dll /
