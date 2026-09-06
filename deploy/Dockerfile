# Build context: greenpng repository root only
#   docker build -f 04-release-github-ci/deploy/Dockerfile -t gv6:latest .
FROM rust:1.88-bookworm AS build
# libz-ng-sys 需要 cmake; openssl-sys 需要 pkg-config/libssl-dev
RUN apt-get update && apt-get install -y --no-install-recommends \
      cmake pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
ENV CARGO_BUILD_JOBS=2
COPY Cargo.toml VERSION ./
COPY 02-probe-analysis/crates ./02-probe-analysis/crates
COPY 02-probe-analysis/modules ./02-probe-analysis/modules
COPY 02-probe-analysis/vendor ./02-probe-analysis/vendor
RUN cargo build --release -p gr-service -p gr-cli -p gr-harden --jobs 2

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=build /src/target/release/gr-service /app/bin/gr-service
COPY --from=build /src/target/release/gr-cli /app/bin/gr-cli
COPY --from=build /src/target/release/gr-harden /app/bin/gr-harden
COPY 02-probe-analysis/panel/admin-spa /app/admin-spa
COPY 02-probe-analysis/probe/fe /app/fe
COPY 02-probe-analysis/spec /app/spec
ENV GR_BIND=0.0.0.0:28680 \
    GV6_BIND=0.0.0.0:28680 \
    GR_PROBE_BIND=0.0.0.0:28765 \
    GV6_PROBE_BIND=0.0.0.0:28765 \
    GR_DATA_DIR=/data \
    GV6_DATA_DIR=/data \
    GR_MODULES_DIR=/modules \
    GV6_MODULES_DIR=/modules \
    GR_ADMIN_SPA=/app/admin-spa \
    GV6_ADMIN_SPA=/app/admin-spa \
    GR_STATIC_DIR=/app/fe \
    GV6_STATIC_DIR=/app/fe \
    GR_SPEC_DIR=/app/spec \
    GV5_SPEC_DIR=/app/spec
EXPOSE 28680 28765
ENTRYPOINT ["/app/bin/gr-service"]
