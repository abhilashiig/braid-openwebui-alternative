# syntax=docker/dockerfile:1.7

FROM node:24-alpine AS web
WORKDIR /web
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend/ ./
RUN npm run build

FROM rust:1-bookworm AS server
WORKDIR /src/backend
COPY backend/ ./
# rust-embed bakes the built SPA into the binary.
COPY --from=web /web/build /src/frontend/build
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/backend/target \
    cargo build --release --locked && cp target/release/braid /braid

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=server /braid /braid
ENV BRAID_BIND=0.0.0.0:3000 \
    BRAID_LOG_FORMAT=json
EXPOSE 3000
ENTRYPOINT ["/braid"]
