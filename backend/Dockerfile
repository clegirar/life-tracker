# Binaire statique : compilation native sur GitHub Actions, croisée sur un Mac, sans émulation dans les deux cas.
FROM --platform=$BUILDPLATFORM rust:1.99.1-alpine@sha256:7e9e2d068a107a2adafd19930011178d14a78326053cd4c3a9f7ad3fd049557d AS build
WORKDIR /src
COPY . .
# rust-toolchain.toml fait foi : rustup l'applique dès qu'on est dans ce dossier.
RUN rustup target add x86_64-unknown-linux-musl
ENV CARGO_PROFILE_RELEASE_STRIP=symbols
# rust-lld seulement en compilation croisée : sur amd64, il lierait aussi les macros procédurales, qui
# ont besoin de la libc du système, et il échoue.
# SQLX_OFFLINE vient de .cargo/config.toml : la compilation lit .sqlx/ et jamais une base.
RUN if [ "$(uname -m)" != x86_64 ]; then export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=rust-lld; fi; \
    cargo build --locked --release --target x86_64-unknown-linux-musl

# Alpine plutôt que scratch : le wget de busybox sert au contrôle de santé.
FROM alpine:3.23.7@sha256:ff7f0c727dd796aa36787bfcd9a64bf35a6ce72a73451b3359fde849b6703d63
COPY --from=build /src/target/x86_64-unknown-linux-musl/release/life-tracker /usr/local/bin/life-tracker
USER 10001:10001
ENTRYPOINT ["life-tracker"]
CMD ["serve"]
