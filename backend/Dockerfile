# Binaire statique : compilation native sur GitHub Actions, croisée sur un Mac, sans émulation dans les deux cas.
FROM --platform=$BUILDPLATFORM rust:1.99.1-alpine@sha256:7e9e2d068a107a2adafd19930011178d14a78326053cd4c3a9f7ad3fd049557d AS build
WORKDIR /src
COPY . .
# rust-toolchain.toml fait foi : rustup l'applique dès qu'on est dans ce dossier.
RUN rustup target add x86_64-unknown-linux-musl
ENV CARGO_PROFILE_RELEASE_STRIP=symbols
# rust-lld seulement en compilation croisée : sur amd64, il lierait aussi les macros procédurales, qui
# ont besoin de la libc du système, et il échoue. De même clang, pour le C et l'assembleur de ring :
# le gcc de l'image ne sait produire que pour sa propre architecture.
# SQLX_OFFLINE vient de .cargo/config.toml : la compilation lit .sqlx/ et jamais une base.
RUN if [ "$(uname -m)" != x86_64 ]; then \
      apk add --no-cache clang; \
      export CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER=rust-lld CC_x86_64_unknown_linux_musl=clang; \
    fi; \
    cargo build --locked --release --target x86_64-unknown-linux-musl
# Le dossier d'état du collecteur : un volume neuf hérite de ce propriétaire. Par COPY, car RUN
# demanderait d'émuler amd64 sur un Mac.
RUN mkdir -p /collector-state

# Alpine plutôt que scratch : le wget de busybox sert au contrôle de santé, et le bundle de certificats
# de la base de l'image à vérifier Last.fm.
FROM alpine:3.23.7@sha256:ff7f0c727dd796aa36787bfcd9a64bf35a6ce72a73451b3359fde849b6703d63
COPY --from=build /src/target/x86_64-unknown-linux-musl/release/life-tracker /usr/local/bin/life-tracker
COPY --from=build --chown=10001:10001 /collector-state /var/lib/life-tracker
USER 10001:10001
ENTRYPOINT ["life-tracker"]
CMD ["serve"]
