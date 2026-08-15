# syntax=docker/dockerfile:1.7

FROM ubuntu:24.04

ARG DEBIAN_FRONTEND=noninteractive
ARG KOHARU_SHA256

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
    ca-certificates \
    fonts-noto-cjk \
    libayatana-appindicator3-1 \
    libgomp1 \
    librsvg2-2 \
    libssl3 \
    libwebkit2gtk-4.1-0 \
    libxdo3 \
    && rm -rf /var/lib/apt/lists/*

COPY dist/koharu /usr/local/bin/koharu
RUN test -n "$KOHARU_SHA256" \
    && echo "$KOHARU_SHA256  /usr/local/bin/koharu" | sha256sum -c - \
    && chmod 0755 /usr/local/bin/koharu

RUN useradd --create-home --shell /bin/bash koharu \
    && install -d -o koharu -g koharu -m 755 /home/koharu/.local/share/Koharu

USER koharu
WORKDIR /home/koharu

VOLUME ["/home/koharu/.local/share/Koharu"]
EXPOSE 4000

CMD ["/usr/local/bin/koharu", "--headless", "--host", "0.0.0.0", "--port", "4000"]
