# Test vectors

`url-normalize.json` is copied verbatim from
`/Users/xu/Desktop/qntx/nostr.js/vectors/core/url-normalize.json`
(repository `qntx/nostr.js`, path `vectors/core/url-normalize.json`).

Provenance, from the file's `source` field:

- generator: `@qntx/nostr`
- version: `0.12.1`
- cross-check: `nostr-tools 2.24.1 @7fa1ef4`: 38/40 agree; `ftp://*` differs
  because this implementation rejects non-ws schemes with `UrlError`.
