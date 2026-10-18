FROM ubuntu:24.04
RUN apt-get update \
 && apt-get install -y --no-install-recommends libegl1 libegl-mesa0 libgles2 libegl-amber0 libgl1-amber-dri \
 && rm -rf /var/lib/apt/lists/*
ENV EGL_PLATFORM=surfaceless
ENV XDG_CACHE_HOME=/tmp/cache
WORKDIR /output
ENTRYPOINT ["/probe/catwalk-wgpu-probe"]
