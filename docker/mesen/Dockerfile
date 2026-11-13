# Mesen 2, built from source at a pinned commit, as the oracle image for
# `gp oracle record` (docs/oracle.md). Mesen is GPL-3.0: it runs here as an
# external program and nothing in the repo links, vendors or commits it.
# This image is separate from the toolchain image (docker/Dockerfile) and has
# its own tag, game-printer-mesen:<commit prefix>-<arch>.
#
# The image holds Mesen and the .NET runtime only. ROMs, input logs and the
# capture script are mounted when a container runs.

# mcr.microsoft.com/dotnet/sdk:8.0-bookworm-slim
ARG SDK_IMAGE=mcr.microsoft.com/dotnet/sdk@sha256:ff39a765f04619c8d146796fc09b38499ae5984f4a2a4c3a0a57d0022855daee
# mcr.microsoft.com/dotnet/runtime:8.0-bookworm-slim
ARG RUNTIME_IMAGE=mcr.microsoft.com/dotnet/runtime@sha256:ac29abc7855ef3dd4697be50ac8baa99151d893d5c4545f48b44579ce2d8f439

# Both base images list Debian bookworm as of this snapshot.debian.org
# timestamp; apt reads the archive at that date, so every package below is
# the version named, whenever the image is built. Snapshots never expire,
# hence Check-Valid-Until off.
ARG DEBIAN_SNAPSHOT=20261020T213255Z

FROM ${SDK_IMAGE} AS build
ARG DEBIAN_SNAPSHOT
ARG TARGETARCH

RUN sed -i \
      -e "s|http://deb.debian.org/debian-security|http://snapshot.debian.org/archive/debian-security/${DEBIAN_SNAPSHOT}|" \
      -e "s|http://deb.debian.org/debian|http://snapshot.debian.org/archive/debian/${DEBIAN_SNAPSHOT}|" \
      /etc/apt/sources.list.d/debian.sources \
 && ! grep -q deb.debian.org /etc/apt/sources.list.d/debian.sources \
 && apt-get -o Acquire::Check-Valid-Until=false update \
 && apt-get install -y --no-install-recommends \
      clang=1:14.0-55.7~deb12u1 \
      make=4.3-4.1 \
      libsdl2-dev=2.26.5+dfsg-1 \
      zip=3.0-13+deb12u1 \
      git=1:2.39.5-0+deb12u3 \
      ca-certificates=20250419~deb12u2 \
 && rm -rf /var/lib/apt/lists/*

# SourMesen/Mesen2 at its final commit (the repository is archived as of
# version 2.1.1; development continues as nesdev-org/MesenCE). git checks the
# commit hash, so the source is pinned by content.
ARG MESEN_COMMIT=b9fa69ddc6d0a331fb103fdb5eef6904305703c2
WORKDIR /src
RUN git init -q mesen \
 && cd mesen \
 && git fetch -q --depth 1 https://github.com/SourMesen/Mesen2.git ${MESEN_COMMIT} \
 && git checkout -q FETCH_HEAD \
 && test "$(git rev-parse HEAD)" = "${MESEN_COMMIT}"

WORKDIR /src/mesen
# Dotnet.Bundle builds macOS app bundles and is referenced at version "*";
# pin it so the restore is the same every time.
RUN sed -i 's|<PackageReference Include="Dotnet.Bundle" Version="\*" />|<PackageReference Include="Dotnet.Bundle" Version="0.9.13" />|' UI/UI.csproj \
 && grep -q 'Dotnet.Bundle" Version="0.9.13"' UI/UI.csproj

# NuGet packages come from a lock file (docker/mesen/nuget/, made in UI/ by
# `dotnet restore -p:RuntimeIdentifier=<rid> -p:Configuration=Release
# -p:OptimizeUi=true -p:RestorePackagesWithLockFile=true`, which matches the
# makefile's publish), and restore runs in locked mode, so a package whose
# version or content hash differs fails the build. The two lock files differ
# only in the runtime identifier.
COPY nuget/ /nuget/
RUN case "${TARGETARCH}" in \
      amd64) rid=linux-x64 ;; \
      arm64) rid=linux-arm64 ;; \
      *) echo "no NuGet lock file for TARGETARCH '${TARGETARCH}' (build with BuildKit and --platform)" >&2; exit 1 ;; \
    esac \
 && cp "/nuget/${rid}.packages.lock.json" UI/packages.lock.json
ENV DOTNET_CLI_TELEMETRY_OPTOUT=1 DOTNET_NOLOGO=1 \
    RestorePackagesWithLockFile=true RestoreLockedMode=true
RUN make -j"$(nproc)" \
 && mkdir -p /out \
 && cp -R bin/linux-*/Release/linux-*/publish/. /out/ \
 && echo "${MESEN_COMMIT}" > /out/MESEN_COMMIT

FROM ${RUNTIME_IMAGE}
ARG DEBIAN_SNAPSHOT
RUN sed -i \
      -e "s|http://deb.debian.org/debian-security|http://snapshot.debian.org/archive/debian-security/${DEBIAN_SNAPSHOT}|" \
      -e "s|http://deb.debian.org/debian|http://snapshot.debian.org/archive/debian/${DEBIAN_SNAPSHOT}|" \
      /etc/apt/sources.list.d/debian.sources \
 && ! grep -q deb.debian.org /etc/apt/sources.list.d/debian.sources \
 && apt-get -o Acquire::Check-Valid-Until=false update \
 && apt-get install -y --no-install-recommends libsdl2-2.0-0=2.26.5+dfsg-1 \
 && rm -rf /var/lib/apt/lists/*
COPY --from=build /out /opt/mesen
# Portable mode: a settings.json next to the program makes Mesen keep its
# home folder there instead of showing the first-run wizard. gp runs the
# container as the calling user, so that folder must be writable by any
# user, and HOME must point somewhere writable.
RUN echo '{}' > /opt/mesen/settings.json \
 && chmod -R a+rwX /opt/mesen
ENV DOTNET_CLI_TELEMETRY_OPTOUT=1 HOME=/tmp
WORKDIR /opt/mesen
ENTRYPOINT ["/opt/mesen/Mesen"]
