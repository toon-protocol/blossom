# blossom

The TOON Protocol **blob app**: a [Blossom](https://github.com/hzrd149/blossom) server behind a
connector route. People pay per KiB to store a blob, through the connector, and anyone reads it for
free by its sha256 hash. Events that use a file name it by that hash in a NIP-92 `imeta` tag.

**Status: design.** There is no code yet. The decisions so far are below and in
[`docs/adr/`](docs/adr/); the vocabulary is in [`GLOSSARY.md`](GLOSSARY.md); the research behind
them is [`docs/research/blob-storage-for-relay.md`](docs/research/blob-storage-for-relay.md).

Like the relay, this is a plain HTTP app. It speaks no ILP and validates no payment: a paid
request reaches it only through the connector, which states the payment on the delivery
(`X-TOON-Payer` / `-Amount` / `-Chain`, connector ADR 0040). All payment validation lives in
[toon-protocol/connector](https://github.com/toon-protocol/connector).

## Design so far

**Paid routes**, reached only through the connector:

| Route | Handler | Price | BUD |
| --- | --- | --- | --- |
| `g.toon.blossom.upload` | `PUT /upload` | base + per KiB | 02 |
| `g.toon.blossom.mirror` | `PUT /mirror` | flat, buys up to a size limit | 04 |
| `g.toon.blossom.media` | `PUT /media` (images only) | base + per KiB of input | 05 |

**Free, on the public host:** blob reads `GET`/`HEAD /<sha256>` (BUD-01), preflight `HEAD /upload`
(BUD-06), delete `DELETE /<sha256>` (BUD-12), list `GET /list/<pubkey>` (BUD-12) and reports
`PUT /report` (BUD-09). Every write needs a kind 24242 authorization (BUD-11), and blob
descriptors carry `nip94` tags (BUD-08). BUD-03 and BUD-10 are client-side. BUD-07 is not
implemented ([ADR 0003](docs/adr/0003-bud-07-is-not-implemented.md)).

**Retention:** one paid upload buys a fixed retention period, and uploading again extends it
([ADR 0002](docs/adr/0002-a-payment-buys-a-fixed-retention-period.md)). Expired holdings are
swept; a blob's bytes go when its last holding does.

**Storage:** a content-addressed directory on a named volume, with metadata in SQLite. An operator
total storage cap, with no default, keeps uploads off until it is set.

**Limits:** the connector caps a paid request at 2 MiB, so v1 accepts blobs up to about 1.9 MiB.

**Moderation:** reports are kept for the operator. A takedown is a startup list of blocked hashes,
as the relay's event blocklist is; there is no admin endpoint.

## License

MIT
