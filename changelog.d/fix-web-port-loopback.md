### Security

- Publish the web page on `127.0.0.1` only. Before, port 1421 was open on every network interface, and
  anyone who could reach it could use the app and its API token. To open it to other machines, set
  `OCR_WEB_BIND=0.0.0.0` and `OCR_WEB_APP_URL` to the address you use. This is a change for anyone who
  opens the page from another machine, such as a server or a remote Docker host.
