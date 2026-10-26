FROM node:24-bookworm-slim AS web
WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY web/ ./
RUN npm run build

FROM rust:1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY --from=web /src/web/dist ./web/dist
RUN cargo build --release

FROM debian:bookworm-slim
# ffmpeg for probing, thumbnails and storyboards; Python for the yt-dlp
# environment the server creates in the data directory on request.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ffmpeg python3 python3-venv ca-certificates \
 && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/hoard /usr/local/bin/hoard
ENV HOARD_DATA_DIR=/data \
    HOARD_LISTEN=0.0.0.0:8150
VOLUME /data
EXPOSE 8150
CMD ["hoard"]
