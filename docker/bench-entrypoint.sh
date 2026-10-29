#!/bin/sh
# Entry point of the benchmark image (docker/Dockerfile.bench). One container reviews one pull
# request mounted the ReviewBench way (/work/repo, /work/pr, RB_OUT) and exits.
#
# It starts, inside the container, what Compose runs as services:
# - a throwaway Postgres on a root-only Unix socket
# - one review runner, keeping /work/repo, /work/pr and /work/out between stages
# - when the benchmark gives no proxy, the provider egress proxy
# unless OCR_BENCH_RUNNER_SOCKETS names runners already running in containers of their own.
# then runs ocr-bench, which plays the server's part. The model CLI still runs only in the
# runner, as UID 10001, under the tool guard.
set -eu

: "${RB_OUT:?RB_OUT is required}"
pg_bin=$(echo /usr/lib/postgresql/*/bin)
host=$(cat /etc/hostname)

install -d -m 0700 /ocr /ocr/data /ocr/auth /ocr/tmp
install -d -m 0700 -o postgres -g postgres /var/lib/ocr-pg /run/ocr-pg
su postgres -s /bin/sh -c "$pg_bin/initdb -D /var/lib/ocr-pg/data -U ocr -A trust -E UTF8 --locale=C --no-sync" >/dev/null
su postgres -s /bin/sh -c "$pg_bin/pg_ctl -D /var/lib/ocr-pg/data -l /var/lib/ocr-pg/log -w start -o \"-c listen_addresses='' -c unix_socket_directories=/run/ocr-pg -c fsync=off -c synchronous_commit=off -c full_page_writes=off\"" >/dev/null
"$pg_bin/createdb" -h /run/ocr-pg -U ocr opencodereview

# ocr-bench runs git as root on a checkout another user owns.
git config --system --add safe.directory '*'
# Benchmarks treat a moved file as new content and anchor findings in it, so the review diff shows
# renamed files as added. RB_CONFIG_DIFF_RENAMES=true keeps git's rename detection.
if [ "${RB_CONFIG_DIFF_RENAMES:-false}" != true ]; then
  git config --system diff.renames false
fi
# Only the frozen snapshot is the model's to read.
chmod 0700 /work/repo 2>/dev/null || true

wait_for() {
  tries=0
  until [ -S "$1" ]; do
    tries=$((tries + 1))
    if [ "$tries" -gt 120 ]; then
      echo "ocr-bench: the review runner $1 did not start" >&2
      exit 1
    fi
    sleep 0.5
  done
}

if [ -n "${OCR_BENCH_RUNNER_SOCKETS:-}" ]; then
  # Runners in containers of their own (bench/adapters run them like Compose's replicas, with
  # their egress proxy): this container only waits for their sockets and reviews through them.
  sockets=$OCR_BENCH_RUNNER_SOCKETS
  for socket in $(echo "$sockets" | tr ',' ' '); do
    wait_for "$socket"
  done
else
  proxy=${HTTPS_PROXY:-${https_proxy:-}}
  if [ -z "$proxy" ]; then
    # Not UID 10001: the runner treats any process of the model UID as a survivor of a stage.
    setpriv --reuid 10002 --regid 10002 --clear-groups ocr-sandbox egress &
    proxy=http://127.0.0.1:8790
  fi

  OCR_RUNNER_KEEP=repo,pr,out \
  OCR_RUNNER_AUTH_ROOT=/ocr/auth/app-managed \
  OCR_RUNNER_PROXY="$proxy" \
    ocr-sandbox runner &

  sockets=/ipc/$host.sock
  wait_for "$sockets"
fi

OCR_DATABASE_URL="host=/run/ocr-pg user=ocr dbname=opencodereview" \
OCR_DATA_DIR=/ocr/data \
OCR_REVIEW_CLAUDE_AUTH_DIR=/ocr/auth/app-managed \
OCR_REVIEW_RUNNER_SOCKETS="$sockets" \
TMPDIR=/ocr/tmp \
  ocr-bench "${1:-reviewbench}"
