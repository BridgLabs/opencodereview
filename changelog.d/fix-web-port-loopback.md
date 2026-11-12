### Security

- **Breaking:** the web page is now published on `127.0.0.1` only. If you open it from another machine
  (a server, a remote Docker context or `DOCKER_HOST`, a LAN), add `OCR_WEB_BIND=0.0.0.0` and
  `OCR_WEB_APP_URL=http://<host>:1421` to `.env`, or keep the default and use
  `ssh -L 1421:127.0.0.1:1421 <host>`. Before, port 1421 was open on every network interface, and
  anyone who could reach it could use the app and its API token.
  Use Docker Engine 28.3.3 or newer (`docker version --format '{{.Server.Version}}'`): older versions can
  expose a port published on `127.0.0.1` to other hosts, so update Docker too.
