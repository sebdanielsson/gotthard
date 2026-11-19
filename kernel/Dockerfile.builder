# Build environment for kernel/build-openwrt.sh.
# Base image pinned by digest, Debian packages pinned to a snapshot date.
FROM debian:trixie-slim@sha256:a72b4433de1cf7d6bae0c4cee74d0547f58601a643b3375f070c51600d63b6b2

ARG DEBIAN_SNAPSHOT=20261031T000000Z

RUN set -eux; \
	rm -f /etc/apt/sources.list.d/debian.sources; \
	printf '%s\n' \
		"deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/${DEBIAN_SNAPSHOT} trixie main" \
		"deb [check-valid-until=no] http://snapshot.debian.org/archive/debian/${DEBIAN_SNAPSHOT} trixie-updates main" \
		"deb [check-valid-until=no] http://snapshot.debian.org/archive/debian-security/${DEBIAN_SNAPSHOT} trixie-security main" \
		>/etc/apt/sources.list; \
	apt-get -o Acquire::Retries=5 update; \
	DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
		bison build-essential bzip2 ca-certificates clang file flex g++ gawk gcc gettext git \
		libelf-dev libgnutls28-dev libncurses-dev libssl-dev python3 python3-dev \
		python3-pyelftools python3-setuptools rsync swig unzip wget xz-utils zlib1g-dev zstd; \
	rm -rf /var/lib/apt/lists/*; \
	useradd -m -u 1001 builder

USER builder
WORKDIR /home/builder
