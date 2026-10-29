# The Imadive server as a container image, made from the release's Linux server archives
# (built and tested by the server job of .github/workflows/desktop.yml), so nothing is
# compiled here. The build context holds them in dist/:
#
#   dist/Imadive-server-<version>-linux-x86_64.tar.gz
#   dist/Imadive-server-<version>-linux-aarch64.tar.gz
#
# Running it: see docs/docker.md.

# Unpacks the archive for the image's platform (on the build machine's own platform, so
# building for ARM needs no emulation).
FROM --platform=$BUILDPLATFORM debian:12-slim AS unpack
ARG TARGETARCH
COPY dist/ /dist/
RUN set -eu; \
    case "$TARGETARCH" in amd64) arch=x86_64 ;; arm64) arch=aarch64 ;; *) echo "no build for $TARGETARCH" >&2; exit 1 ;; esac; \
    mkdir -p /out/opt/imadive /out/data /out/photos; \
    tar -xzf /dist/Imadive-server-*-linux-"$arch".tar.gz -C /out/opt/imadive --strip-components=1; \
    test -x /out/opt/imadive/imadive
# libheif, for iPhone photos (HEIC), with the decoders and zlib it needs (the image has glibc
# and the C++ runtime): Debian 12's packages for the image's platform, downloaded and unpacked
# here (no emulation), their libraries copied below.
RUN set -eu; \
    dpkg --add-architecture "$TARGETARCH"; \
    apt-get update -qq; \
    mkdir -p /debs /out/heif; cd /debs; \
    for p in libheif1 libde265-0 libaom3 libdav1d6 libx265-199 libnuma1 zlib1g; do apt-get download -qq "$p:$TARGETARCH"; done; \
    for d in *.deb; do dpkg-deb -x "$d" /out/heif; done; \
    mkdir -p /out/heif/lib; \
    ls /out/heif/usr/lib/*-linux-gnu/libheif.so.1

# Only what the program needs: glibc and the C++ runtime that ONNX Runtime uses. No shell,
# no package manager; it runs as an unprivileged user (65532).
FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=unpack /out/opt/imadive /opt/imadive
# zlib's package installs into /lib, the others into /usr/lib.
COPY --from=unpack /out/heif/usr/lib/ /usr/lib/
COPY --from=unpack /out/heif/lib/ /lib/
COPY --from=unpack --chown=65532:65532 /out/data /data
COPY --from=unpack /out/photos /photos

# The repository's address (org.opencontainers.image.source, which links the package to it on
# GitHub), version and revision are added by the workflow, from wherever the repository is.
LABEL org.opencontainers.image.title="Imadive" \
      org.opencontainers.image.description="Fast local photo and video gallery: timeline, places, upcoming anniversaries and people found by face recognition" \
      org.opencontainers.image.licenses="AGPL-3.0-only"

# The index (and its thumbnails) lives in /data; the photos are mounted at /photos (or
# anywhere, given as the command). Other options go before the folders, for example
# `--allow-host photos.home /photos`.
# Settings says to update with `docker pull` (a container can't replace its own image).
ENV IMADIVE_INSTALL=docker
VOLUME ["/data"]
EXPOSE 7878
WORKDIR /data
ENTRYPOINT ["/opt/imadive/imadive", "--host", "0.0.0.0", "--port", "7878", "--data", "/data", \
            "--models", "/opt/imadive/models", "--onnxruntime", "/opt/imadive/onnxruntime"]
CMD ["/photos"]
