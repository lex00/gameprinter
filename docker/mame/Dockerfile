# MAME from Debian's own package, as the oracle image for arcade sources
# (docs/oracle-mame.md). MAME is GPL-2.0+ as a whole: it runs here as an
# external program, and nothing in the repo links, vendors or commits it.
# The tag is gameprinter-mame:<MAME version>-<arch>, set by `just mame-build`.
#
# The image holds MAME and the libraries it needs, nothing from any game.
# The romset, the input log and the capture script are mounted when a
# container runs, and the romset only read-only.

# debian:trixie-slim, the multi-arch index as of 2026-11-04.
ARG BASE_IMAGE=debian@sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2

# The base image says it was made from this snapshot.debian.org timestamp
# (the comments in its /etc/apt/sources.list.d/debian.sources). apt reads
# the archive as it was then, so every package below is the version named,
# whenever the image is built, and so are their dependencies. Snapshots
# never expire, hence Check-Valid-Until off.
ARG DEBIAN_SNAPSHOT=20261104T212339Z

# Debian 13's MAME. rbtapper has been in MAME since long before 0.276, and
# this version runs it (docs/oracle-mame.md, "The image").
ARG MAME_DEB_VERSION=0.276+dfsg.1-1+deb13u1

FROM ${BASE_IMAGE}
ARG DEBIAN_SNAPSHOT
ARG MAME_DEB_VERSION

RUN sed -i \
      -e "s|http://deb.debian.org/debian-security|http://snapshot.debian.org/archive/debian-security/${DEBIAN_SNAPSHOT}|" \
      -e "s|http://deb.debian.org/debian|http://snapshot.debian.org/archive/debian/${DEBIAN_SNAPSHOT}|" \
      /etc/apt/sources.list.d/debian.sources \
 && ! grep -q '^URIs: http://deb.debian.org' /etc/apt/sources.list.d/debian.sources \
 && apt-get -o Acquire::Check-Valid-Until=false update \
 && apt-get install -y --no-install-recommends \
      mame=${MAME_DEB_VERSION} \
      mame-data=${MAME_DEB_VERSION} \
 && rm -rf /var/lib/apt/lists/* \
 && /usr/games/mame -version > /MAME_VERSION

# No display and no sound card: SDL gets its dummy drivers, and every run
# passes -video none and -sound none as well. The container runs as the
# calling user, so HOME points somewhere any user can write.
ENV SDL_VIDEODRIVER=dummy SDL_AUDIODRIVER=dummy HOME=/tmp
WORKDIR /tmp
ENTRYPOINT ["/usr/games/mame"]
