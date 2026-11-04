# cairn-net: built in a Rust image (the host's Debian 12 cargo is too old),
# run on the same Debian release as the builder (libssl3).
FROM rust:1.85-slim-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY . .
RUN cargo test --release --locked && cargo build --release --locked

FROM debian:bookworm-slim
# git + ssh: git mode (clone and push of the DHCP file with the app's deploy key)
RUN apt-get update && apt-get install -y --no-install-recommends libssl3 git openssh-client ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/cairn-net /usr/local/bin/cairn-net
USER nobody
# CMD apart so that `docker run --rm -i <image> hash-password` works
ENTRYPOINT ["/usr/local/bin/cairn-net"]
CMD ["/etc/cairn-net/config.json"]
