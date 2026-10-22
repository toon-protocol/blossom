# blossom

The TOON Protocol **blob app**: a [Blossom](https://github.com/hzrd149/blossom) server behind a
connector route. People pay per KiB to store a blob, through the connector, and anyone reads it for
free by its sha256 hash. Events that use a file name it by that hash in a NIP-92 `imeta` tag.

**Status: design.** There is no code yet. The decisions are below and in
[`docs/adr/`](docs/adr/); the vocabulary is in [`GLOSSARY.md`](GLOSSARY.md); the research behind
them is [`docs/research/blob-storage-for-relay.md`](docs/research/blob-storage-for-relay.md).

Like the relay, this is a plain HTTP app. It speaks no ILP and validates no payment: a paid
request reaches it only through the connector, which states the payment on the delivery
(`X-TOON-Payer` / `-Amount` / `-Chain`, connector ADR 0040). The blob app records that statement
and trusts it. All payment validation lives in
[toon-protocol/connector](https://github.com/toon-protocol/connector).

## Design

### Paid routes

Reached only through the connector. Devnet prices are in the settlement asset's smallest unit.

| Route | Handler | Devnet price | BUD |
| --- | --- | --- | --- |
| `g.toon.blossom.upload` | `PUT /upload` | `{ base = 1000, per_kib = 10 }` | 02 |
| `g.toon.blossom.mirror` | `PUT /mirror` | flat 25,000, buys a blob of up to 1.9 MiB | 04 |
| `g.toon.blossom.media` | `PUT /media`, images only | `{ base = 1000, per_kib = 10 }` of input | 05 |

Media optimization strips metadata and re-encodes, and keeps only the output. The original is
never stored. Mirror fetches only public `https` addresses, checked on the resolved address and on
every redirect, so it can never reach a service on the bundle's internal network.

### Free, on the public host

| Request | BUD |
| --- | --- |
| `GET` / `HEAD /<sha256>`: blob reads | 01 |
| `HEAD /upload`: preflight, running exactly the paid path's checks | 06 |
| `DELETE /<sha256>`: ends the owner's holding | 12 |
| `GET /list/<pubkey>` | 12 |
| `PUT /report`: a NIP-56 report, kept for the operator | 09 |

A `PUT` to a paid path sent directly to the public host is refused with `403`: uploads go through TOON.
Every write carries a kind 24242 authorization (BUD-11), and blob descriptors carry `nip94` tags
(BUD-08). BUD-03 and BUD-10 are client-side. BUD-07 is not implemented
([ADR 0003](docs/adr/0003-bud-07-is-not-implemented.md)).

### Holdings and retention

The signer of the upload's authorization is the blob's **owner**; the connector's payer is recorded
but is not an identity. Each upload is a **holding** with one owner and one expiry. Identical bytes
are kept once, for as long as any holding on them is live, so one owner's delete never ends
another's holding.

One paid upload buys a fixed **retention period** (30 days on devnet). Uploading again extends the
holding to max(now, current expiry) plus the period. The expiry is advertised with `Sunset`
([ADR 0002](docs/adr/0002-a-payment-buys-a-fixed-retention-period.md)). Expired holdings are
swept, and a blob's bytes go with its last holding. The sweep interval is set with
`TOON_BLOB_SWEEP_INTERVAL_SECS` (default 1 hour) and the period with `TOON_BLOB_RETENTION_SECS`;
a blob whose last holding has expired is `404` even before the sweep runs.

A **blob reference** in an event is data only: nothing is enforced, and nothing is kept alive
because an event names it.

### Limits and moderation

- The connector caps a paid request at 2 MiB, so blobs go up to about 1.9 MiB.
- An operator total storage cap (`TOON_BLOB_STORAGE_CAP_BYTES`), with no default, keeps uploads off
  until it is set. Settings: `TOON_BLOB_MAX_BLOB_BYTES` (default about 1.9 MiB, over it is `413`) and
  `TOON_BLOB_ALLOWED_CONTENT_TYPES` (optional, comma-separated, `415` outside it); each has a flag.
  Cap refusals are `507`. The preflight and the paid upload run one acceptance check.
- Reports never remove anything. A takedown is `TOON_BLOCKED_BLOBS`, a list of blocked
  hashes read at startup (a malformed entry fails startup), as the relay's event blocklist is: it ends every holding, and the preflight and upload
  refuse the hash. There is no admin endpoint.

A report is a signed kind 1984 event with an `x` tag holding the blob's sha256 (and, as the tag's
third element, the report type). A bad signature, the wrong kind or no hash is refused with `400`,
and a body over 64 KiB with `413`. An accepted report is kept in the `reports` table of
`blossom.db` in the storage directory, one row per reported blob, with the reporter's pubkey, the
type, the content, the receipt time and the event as sent. The image has no `sqlite3`, so the
operator reads them with a `sqlite3` outside it, against the volume's copy of the database:
`sqlite3 <volume dir>/blossom.db 'SELECT sha256, report_type, reporter, content FROM reports ORDER BY received_at'`
(`docker volume inspect` gives the directory; the database is in WAL mode, so a copy of it needs
`blossom.db-wal` too). To act, they add the hash to `TOON_BLOCKED_BLOBS`.

### Storage

A content-addressed directory on a named volume, with metadata in SQLite. No S3 server until there
is a second box; then Garage or SeaweedFS, not MinIO.

## Deployment

Two modes, the same as the store's:

- **Its own bundle**: `deploy/docker-compose.yml` stands up a connector (on an immutable pin of the
  stock image), the blob app and Caddy. Its read-only `deploy/connector.toml` carries the three
  routes at their devnet prices, each pointing at its handler on the write port. Blobs are kept on
  the named volume `blob_data`. Only Caddy publishes ports; the write port is never published, and
  `deploy/Caddyfile` proxies only the public port's methods and paths (and `/ilp` to the connector).
- **Shared edge**: `deploy/docker-compose.shared-edge.yml` turns the blob app's own connector (and
  Caddy) off and joins an `edge-blossom` network, leaving the blob app's image and settings alone.
  The host bundle's connector joins that network, and its `connector.toml` and Caddyfile carry the
  blob app's routes and public host.

`tests/deploy_guard.rs` checks these files.

The blob app runs unchanged in either mode.

## Build and release

Rust, on the relay's toolchain pin and `docs/rust-coding-standards.md`. CI builds the image once
and runs two suites against that image: the app's own black-box suite (holdings, retention, the
`403`, the mirror guard, takedown, the storage cap, the preflight) and
[blossom-compliance](https://github.com/v0l/blossom-compliance) through a test-only proxy that adds a
stated payment to the paid paths. Pull requests build and run both suites but never publish. On a
push to `main` where the gate and both suites are green, the workflow pushes that same image to
`ghcr.io/toon-protocol/blossom` as an immutable `:sha-<full commit SHA>` tag and, if that commit is
still `main`'s head, the moving `:release` tag. A red suite publishes nothing.

## License

MIT
