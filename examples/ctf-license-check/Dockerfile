FROM rust:1-bookworm AS build

WORKDIR /src
COPY . .
RUN cargo build --workspace --release --locked
RUN target/release/slopium \
    --manifest-path examples/ctf-license-check/Slopium.toml \
    build --release

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install --yes --no-install-recommends socat \
    && rm -rf /var/lib/apt/lists/*

COPY --from=build \
    /src/examples/ctf-license-check/target/x86_64-unknown-linux-gnu/release/slopium-license \
    /opt/challenge/slopium-license

# FLAG is deliberately not baked in: an ENV here would be committed to an image
# layer and readable by anyone who pulls the image. Supply it at run time
# (compose passes it through, or use the platform's secret store).
EXPOSE 31337
USER 65534:65534

# max-children bounds concurrent connections so a flood cannot exhaust the
# container's process table.
CMD ["socat", "TCP-LISTEN:31337,reuseaddr,fork,max-children=64", "EXEC:/opt/challenge/slopium-license,stderr"]
