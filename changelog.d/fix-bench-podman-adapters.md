### Fixed

- Run the benchmark adapters and `docker/bench-smoke.py` on rootless Podman with SELinux enforcing:
  set `OCR_CONTAINER_ENGINE=podman`, as for the other Docker scripts. Before, they called `docker`
  directly, and on an SELinux host the container could not read its bind mounts.
